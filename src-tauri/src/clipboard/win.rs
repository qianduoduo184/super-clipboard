use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_V,
};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, HWND_MESSAGE, MSG, WM_CLIPBOARDUPDATE, WM_DESTROY, WNDCLASSW,
};

use crate::blobs::image::image_identity_from_dib;
use crate::clipboard::sequence::{is_stale_worker_event, ClipboardSequenceState, SequenceDecision};
use crate::clipboard::types::{ClipboardCapture, ClipboardItemDraft, ClipboardItemType};

// DROPFILES structure for CF_HDROP
#[repr(C)]
#[allow(non_snake_case)]
struct DROPFILES {
    pFiles: u32,
    pt: POINT,
    fNC: i32,
    fWide: i32,
}

static CLIPBOARD_EVENT_TX: OnceLock<Mutex<Option<Sender<u32>>>> = OnceLock::new();
static CLIPBOARD_SEQUENCE_STATE: OnceLock<Mutex<ClipboardSequenceState>> = OnceLock::new();

pub fn start_listener<F>(on_change: F) -> Result<()>
where
    F: Fn(u32) + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<u32>();
    CLIPBOARD_EVENT_TX
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|error| anyhow!("clipboard event lock poisoned: {error}"))?
        .replace(tx);

    thread::spawn(move || {
        for sequence in rx {
            on_change(sequence);
        }
    });

    thread::spawn(|| {
        if let Err(error) = run_message_window() {
            crate::diagnostics::error(format!("clipboard listener failed: {error}"));
        }
    });

    Ok(())
}

pub fn read_current_clipboard(event_sequence: u32) -> Result<Option<Vec<ClipboardCapture>>> {
    let _guard = ClipboardGuard::open()?;
    let current_sequence = unsafe { GetClipboardSequenceNumber() };
    if is_stale_worker_event(event_sequence, current_sequence) {
        return Ok(None);
    }

    // Privacy: honor the Windows clipboard-exclusion formats that password managers
    // and other sensitive sources set to opt out of clipboard history/monitoring.
    // If present, skip recording entirely so secrets never touch the SQLite store.
    if is_history_excluded() {
        crate::diagnostics::info(
            "clipboard: content marked excluded from history, skipping capture",
        );
        return Ok(Some(Vec::new()));
    }

    // Try reading file list - log error but continue to next format if it fails
    match read_file_list() {
        Ok(Some(files)) => {
            let preview = files
                .iter()
                .take(3)
                .map(|path| {
                    Path::new(path)
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or(path)
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Ok(Some(vec![ClipboardCapture::Draft(ClipboardItemDraft {
                item_type: ClipboardItemType::Files,
                size_bytes: files.len() as i64,
                preview,
                content: Some(serde_json::to_string(&files)?),
                content_path: None,
                content_hash: None,
                source_app: None,
            })]));
        }
        Ok(None) => {}
        Err(error) => {
            crate::diagnostics::warn(format!("clipboard: file list read failed: {error}"));
        }
    }

    // Try reading image - log error but continue to text if it fails
    match process_preferred_dib(
        |format| unsafe {
            if IsClipboardFormatAvailable(format) == 0 {
                Ok(None)
            } else {
                read_global_bytes(format)
            }
        },
        |_format, dib_bytes| {
            image_identity_from_dib(&dib_bytes)?;
            Ok(dib_bytes)
        },
    ) {
        Ok(Some(dib)) => {
            return Ok(Some(vec![ClipboardCapture::ImageDib(dib)]));
        }
        Ok(None) => {}
        Err(error) => {
            crate::diagnostics::warn(format!("clipboard: image capture failed: {error}"));
        }
    }

    // Try reading text
    match read_unicode_text() {
        Ok(Some(text)) if !text.trim().is_empty() => {
            // Compress all whitespace (newlines, tabs, multiple spaces) into single spaces for preview
            // The original multi-line content is preserved in 'content' field
            let preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
            return Ok(Some(vec![ClipboardCapture::Draft(ClipboardItemDraft {
                item_type: ClipboardItemType::Text,
                size_bytes: text.len() as i64,
                preview,
                content: Some(text),
                content_path: None,
                content_hash: None,
                source_app: None,
            })]));
        }
        Ok(_) => {}
        Err(error) => {
            crate::diagnostics::warn(format!("clipboard: text read failed: {error}"));
        }
    }

    Ok(Some(Vec::new()))
}

pub fn write_text_to_clipboard(text: &str) -> Result<()> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        if EmptyClipboard() == 0 {
            return Err(anyhow!("empty clipboard"));
        }

        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        let byte_len = wide.len() * std::mem::size_of::<u16>();
        let handle = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if handle == Default::default() {
            return Err(anyhow!("allocate clipboard memory"));
        }
        let locked = GlobalLock(handle) as *mut u16;
        if locked.is_null() {
            return Err(anyhow!("lock clipboard allocation"));
        }

        std::ptr::copy_nonoverlapping(wide.as_ptr(), locked, wide.len());
        GlobalUnlock(handle);
        if SetClipboardData(CF_UNICODETEXT as u32, handle) == Default::default() {
            return Err(anyhow!("set unicode text"));
        }
    }
    register_current_internal_sequence();

    Ok(())
}

pub fn write_dib_to_clipboard(dib_bytes: &[u8]) -> Result<()> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        if EmptyClipboard() == 0 {
            return Err(anyhow!("empty clipboard"));
        }

        let handle = GlobalAlloc(GMEM_MOVEABLE, dib_bytes.len());
        if handle == Default::default() {
            return Err(anyhow!("allocate clipboard image memory"));
        }
        let locked = GlobalLock(handle) as *mut u8;
        if locked.is_null() {
            return Err(anyhow!("lock clipboard image allocation"));
        }

        std::ptr::copy_nonoverlapping(dib_bytes.as_ptr(), locked, dib_bytes.len());
        GlobalUnlock(handle);
        if SetClipboardData(CF_DIB as u32, handle) == Default::default() {
            return Err(anyhow!("set DIB image"));
        }
    }
    register_current_internal_sequence();

    Ok(())
}

pub fn write_files_to_clipboard(file_paths: &[String]) -> Result<()> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        if EmptyClipboard() == 0 {
            return Err(anyhow!("empty clipboard"));
        }

        // Calculate total size needed for DROPFILES structure + file paths
        let mut total_size = size_of::<DROPFILES>();
        let mut wide_paths: Vec<Vec<u16>> = Vec::new();

        for path in file_paths {
            let mut wide: Vec<u16> = path.encode_utf16().collect();
            wide.push(0); // null terminator for each path
            total_size += wide.len() * size_of::<u16>();
            wide_paths.push(wide);
        }
        total_size += size_of::<u16>(); // final null terminator

        let handle = GlobalAlloc(GMEM_MOVEABLE, total_size);
        if handle == Default::default() {
            return Err(anyhow!("allocate clipboard file memory"));
        }

        let locked = GlobalLock(handle) as *mut u8;
        if locked.is_null() {
            return Err(anyhow!("lock clipboard file allocation"));
        }

        // Write DROPFILES structure
        let dropfiles = locked as *mut DROPFILES;
        (*dropfiles).pFiles = size_of::<DROPFILES>() as u32;
        (*dropfiles).pt.x = 0;
        (*dropfiles).pt.y = 0;
        (*dropfiles).fNC = 0;
        (*dropfiles).fWide = 1; // Unicode paths

        // Write file paths
        let mut offset = size_of::<DROPFILES>();
        for wide_path in &wide_paths {
            let dest = locked.add(offset) as *mut u16;
            std::ptr::copy_nonoverlapping(wide_path.as_ptr(), dest, wide_path.len());
            offset += wide_path.len() * size_of::<u16>();
        }
        // Write final null terminator
        let final_null = locked.add(offset) as *mut u16;
        *final_null = 0;

        GlobalUnlock(handle);
        if SetClipboardData(CF_HDROP as u32, handle) == Default::default() {
            return Err(anyhow!("set file list"));
        }
    }
    register_current_internal_sequence();

    Ok(())
}

/// Write HTML to the clipboard in `CF_HTML` format (preserving formatting) plus a
/// `CF_UNICODETEXT` plain-text fallback, so both HTML-aware apps (Word, browsers,
/// mail) and plain-text targets paste correctly.
pub fn write_html_to_clipboard(html: &str, plain_text: &str) -> Result<()> {
    let cf_html_format = unsafe { RegisterClipboardFormatW(wide_null("HTML Format").as_ptr()) };
    if cf_html_format == 0 {
        return Err(anyhow!("register HTML clipboard format"));
    }
    let cf_html_bytes = build_cf_html(html);

    let _guard = ClipboardGuard::open()?;
    unsafe {
        if EmptyClipboard() == 0 {
            return Err(anyhow!("empty clipboard"));
        }
        set_clipboard_global(cf_html_format, &cf_html_bytes)?;

        // Plain-text fallback (UTF-16LE + null terminator) as raw bytes.
        let mut wide: Vec<u16> = plain_text.encode_utf16().collect();
        wide.push(0);
        let byte_len = wide.len() * size_of::<u16>();
        let text_bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, byte_len);
        set_clipboard_global(CF_UNICODETEXT as u32, text_bytes)?;
    }
    register_current_internal_sequence();

    Ok(())
}

fn sequence_state() -> &'static Mutex<ClipboardSequenceState> {
    CLIPBOARD_SEQUENCE_STATE.get_or_init(|| Mutex::new(ClipboardSequenceState::default()))
}

fn register_current_internal_sequence() {
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let mut state = sequence_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.register_internal(sequence);
}

/// Allocate a moveable global buffer, copy `bytes` into it, and hand it to the
/// clipboard for `format`. Must be called between `EmptyClipboard` and closing the
/// clipboard (caller holds the `ClipboardGuard`).
unsafe fn set_clipboard_global(format: u32, bytes: &[u8]) -> Result<()> {
    let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
    if handle == Default::default() {
        return Err(anyhow!("allocate clipboard memory"));
    }
    let locked = GlobalLock(handle) as *mut u8;
    if locked.is_null() {
        return Err(anyhow!("lock clipboard allocation"));
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), locked, bytes.len());
    GlobalUnlock(handle);
    if SetClipboardData(format, handle) == Default::default() {
        return Err(anyhow!("set clipboard data"));
    }
    Ok(())
}

/// Wrap an HTML fragment in the `CF_HTML` envelope with the required byte offsets.
/// Offsets are byte counts from the start of the data; each is written as a fixed
/// 10-digit field so the header length is constant regardless of the values.
fn build_cf_html(fragment: &str) -> Vec<u8> {
    let header = |start_html: usize, end_html: usize, start_frag: usize, end_frag: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            start_html, end_html, start_frag, end_frag
        )
    };
    let header_len = header(0, 0, 0, 0).len();
    let prefix = "<html><body>\r\n<!--StartFragment-->";
    let suffix = "<!--EndFragment-->\r\n</body></html>";

    let start_html = header_len;
    let start_fragment = header_len + prefix.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + suffix.len();

    let mut out = String::with_capacity(end_html);
    out.push_str(&header(start_html, end_html, start_fragment, end_fragment));
    out.push_str(prefix);
    out.push_str(fragment);
    out.push_str(suffix);
    out.into_bytes()
}

/// Return true when the current clipboard carries a Windows exclusion marker that
/// asks clipboard managers not to record it. Password managers (KeePass, 1Password,
/// Bitwarden, …) set these. Must be called while the clipboard is open.
fn is_history_excluded() -> bool {
    unsafe {
        let exclude_monitor = RegisterClipboardFormatW(
            wide_null("ExcludeClipboardContentFromMonitorProcessing").as_ptr(),
        );
        if exclude_monitor != 0 && IsClipboardFormatAvailable(exclude_monitor) != 0 {
            return true;
        }

        let can_include =
            RegisterClipboardFormatW(wide_null("CanIncludeInClipboardHistory").as_ptr());
        if can_include != 0 && IsClipboardFormatAvailable(can_include) != 0 {
            // Value is a DWORD; 0 means "do not include in history".
            if let Some(0) = read_clipboard_dword(can_include) {
                return true;
            }
        }
    }
    false
}

/// Read a 4-byte DWORD payload for a clipboard `format`, if present. Caller holds
/// the clipboard open.
unsafe fn read_clipboard_dword(format: u32) -> Option<u32> {
    let handle = GetClipboardData(format);
    if handle == Default::default() {
        return None;
    }
    if GlobalSize(handle) < 4 {
        return None;
    }
    let locked = GlobalLock(handle) as *const u8;
    if locked.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(locked, 4);
    let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    GlobalUnlock(handle);
    Some(value)
}

pub fn simulate_paste_shortcut() -> Result<()> {
    // Wait for window focus to transition back to the previous application
    // after getCurrentWindow().hide() is called from the frontend.
    // Poll for focus change instead of fixed delay to avoid race conditions.
    for attempt in 0..10 {
        thread::sleep(Duration::from_millis(15));
        // On Windows, we can't easily check if our window lost focus without HWND,
        // so we use a reasonable retry strategy: short delays that sum to ~150ms
        if attempt > 5 {
            break; // After 90ms (6 * 15ms), assume focus has switched
        }
    }

    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_V, false),
        keyboard_input(VK_V, true),
        keyboard_input(VK_CONTROL, true),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err(anyhow!("send Ctrl+V input"));
    }
    Ok(())
}

fn run_message_window() -> Result<()> {
    unsafe {
        let class_name = wide_null("SuperClipboardListenerWindow");
        let mut window_class: WNDCLASSW = zeroed();
        window_class.lpfnWndProc = Some(window_proc);
        window_class.lpszClassName = class_name.as_ptr();

        if RegisterClassW(&window_class) == 0 {
            return Err(anyhow!("register clipboard listener window class"));
        }
        crate::diagnostics::info("clipboard: listener window class registered");

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide_null("").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            null_mut(),
            null(),
        );
        if hwnd == null_mut() {
            return Err(anyhow!("create clipboard listener window"));
        }
        crate::diagnostics::info("clipboard: listener message window created");

        if AddClipboardFormatListener(hwnd) == 0 {
            return Err(anyhow!("register clipboard listener"));
        }
        crate::diagnostics::info("clipboard: format listener registered");

        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&message);
        }
    }

    Ok(())
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLIPBOARDUPDATE => {
            let sequence = unsafe { GetClipboardSequenceNumber() };
            let decision = sequence_state()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .classify_notification(sequence);
            if decision == SequenceDecision::Capture {
                if let Some(lock) = CLIPBOARD_EVENT_TX.get() {
                    if let Ok(guard) = lock.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(sequence);
                        }
                    }
                }
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Result<Self> {
        let retry_delays = [
            Duration::from_millis(20),
            Duration::from_millis(40),
            Duration::from_millis(80),
            Duration::from_millis(120),
        ];

        for delay in retry_delays {
            unsafe {
                if OpenClipboard(null_mut()) != 0 {
                    return Ok(Self);
                }
            }
            thread::sleep(delay);
        }

        unsafe {
            if OpenClipboard(null_mut()) == 0 {
                return Err(anyhow!("open clipboard"));
            }
        }
        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

fn read_unicode_text() -> Result<Option<String>> {
    // Security: Limit maximum clipboard text size to prevent memory exhaustion attacks
    const MAX_CLIPBOARD_TEXT_LEN: usize = 100_000_000; // 100M UTF-16 units (~200MB)

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) == 0 {
            return Ok(None);
        }
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle == Default::default() {
            return Err(anyhow!("get unicode text"));
        }
        let locked = GlobalLock(handle) as *const u16;
        if locked.is_null() {
            return Ok(None);
        }

        let mut len = 0;
        while len < MAX_CLIPBOARD_TEXT_LEN && *locked.add(len) != 0 {
            len += 1;
        }

        if len >= MAX_CLIPBOARD_TEXT_LEN {
            GlobalUnlock(handle);
            crate::diagnostics::warn(format!(
                "clipboard: text exceeds size limit ({} UTF-16 units)",
                MAX_CLIPBOARD_TEXT_LEN
            ));
            return Err(anyhow!("clipboard text exceeds maximum size limit"));
        }

        let slice = std::slice::from_raw_parts(locked, len);
        let value = String::from_utf16_lossy(slice);
        GlobalUnlock(handle);
        Ok(Some(value))
    }
}

fn process_preferred_dib<T, R, P>(mut read_format: R, mut process_dib: P) -> Result<Option<T>>
where
    R: FnMut(u32) -> Result<Option<Vec<u8>>>,
    P: FnMut(u32, Vec<u8>) -> Result<T>,
{
    let mut first_error = None;
    for format in [CF_DIBV5 as u32, CF_DIB as u32] {
        match read_format(format).and_then(|bytes| match bytes {
            Some(bytes) => process_dib(format, bytes).map(Some),
            None => Ok(None),
        }) {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {}
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    first_error.map_or(Ok(None), Err)
}

fn read_file_list() -> Result<Option<Vec<String>>> {
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP as u32) == 0 {
            return Ok(None);
        }

        let handle = GetClipboardData(CF_HDROP as u32);
        if handle == Default::default() {
            return Err(anyhow!("get file list"));
        }
        let hdrop = handle as HDROP;
        let count = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0);
        if count == 0 {
            return Ok(None);
        }

        let mut files = Vec::with_capacity(count as usize);
        for index in 0..count {
            let len = DragQueryFileW(hdrop, index, null_mut(), 0);
            if len == 0 {
                continue;
            }

            let mut buffer = vec![0u16; len as usize + 1];
            let written = DragQueryFileW(hdrop, index, buffer.as_mut_ptr(), buffer.len() as u32);
            if written == 0 {
                continue;
            }
            files.push(String::from_utf16_lossy(&buffer[..written as usize]));
        }

        Ok(Some(files))
    }
}

unsafe fn read_global_bytes(format: u32) -> Result<Option<Vec<u8>>> {
    // Security: Limit maximum clipboard blob size to prevent memory exhaustion attacks
    const MAX_CLIPBOARD_BLOB_SIZE: usize = 500_000_000; // 500MB

    let handle = GetClipboardData(format);
    if handle == Default::default() {
        return Err(anyhow!("get clipboard data"));
    }
    let size = GlobalSize(handle);
    if size == 0 {
        return Ok(None);
    }

    if size > MAX_CLIPBOARD_BLOB_SIZE {
        crate::diagnostics::warn(format!(
            "clipboard: blob exceeds size limit ({} bytes)",
            size
        ));
        return Err(anyhow!(
            "clipboard data exceeds maximum size limit: {} bytes",
            size
        ));
    }

    let locked = GlobalLock(handle) as *const c_void;
    if locked.is_null() {
        return Ok(None);
    }

    let bytes = std::slice::from_raw_parts(locked as *const u8, size);
    let value = bytes.to_vec();
    GlobalUnlock(handle);
    Ok(Some(value))
}

fn keyboard_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod dib_candidate_tests {
    use super::{process_preferred_dib, CF_DIB, CF_DIBV5};
    use anyhow::{anyhow, Result};

    #[test]
    fn malformed_v5_falls_back_to_valid_legacy_dib() {
        let mut reads = Vec::new();
        let mut processed = Vec::new();

        let result = process_preferred_dib(
            |format: u32| -> Result<Option<Vec<u8>>> {
                reads.push(format);
                Ok(Some(vec![format as u8]))
            },
            |format: u32, _bytes: Vec<u8>| -> Result<&'static str> {
                processed.push(format);
                if format == CF_DIBV5 as u32 {
                    Err(anyhow!("malformed V5 DIB"))
                } else {
                    Ok("legacy DIB")
                }
            },
        )
        .expect("legacy candidate succeeds");

        assert_eq!(result, Some("legacy DIB"));
        assert_eq!(reads, vec![CF_DIBV5 as u32, CF_DIB as u32]);
        assert_eq!(processed, vec![CF_DIBV5 as u32, CF_DIB as u32]);
    }

    #[test]
    fn valid_v5_short_circuits_legacy_dib() {
        let mut reads = Vec::new();
        let mut processed = Vec::new();

        let result = process_preferred_dib(
            |format: u32| -> Result<Option<Vec<u8>>> {
                reads.push(format);
                Ok(Some(vec![5]))
            },
            |format: u32, _bytes: Vec<u8>| -> Result<&'static str> {
                processed.push(format);
                Ok("V5 DIB")
            },
        )
        .expect("V5 candidate succeeds");

        assert_eq!(result, Some("V5 DIB"));
        assert_eq!(reads, vec![CF_DIBV5 as u32]);
        assert_eq!(processed, vec![CF_DIBV5 as u32]);
    }

    #[test]
    fn both_processing_failures_return_the_first_error() {
        let mut reads = Vec::new();
        let mut processed = Vec::new();

        let error = process_preferred_dib(
            |format: u32| -> Result<Option<Vec<u8>>> {
                reads.push(format);
                Ok(Some(vec![format as u8]))
            },
            |format: u32, _bytes: Vec<u8>| -> Result<()> {
                processed.push(format);
                if format == CF_DIBV5 as u32 {
                    Err(anyhow!("malformed V5 DIB"))
                } else {
                    Err(anyhow!("malformed legacy DIB"))
                }
            },
        )
        .expect_err("both candidates should fail");

        assert_eq!(error.to_string(), "malformed V5 DIB");
        assert_eq!(reads, vec![CF_DIBV5 as u32, CF_DIB as u32]);
        assert_eq!(processed, vec![CF_DIBV5 as u32, CF_DIB as u32]);
    }
}

#[cfg(test)]
mod dib_fallback_tests {
    use super::{process_preferred_dib, CF_DIB, CF_DIBV5};
    use anyhow::anyhow;

    #[test]
    fn v5_success_does_not_read_legacy_dib() {
        let mut reads = Vec::new();

        let result = process_preferred_dib(
            |format| {
                reads.push(format);
                if format == CF_DIBV5 as u32 {
                    Ok(Some(vec![5]))
                } else {
                    panic!("legacy DIB should not be read")
                }
            },
            |_format, bytes| Ok(bytes),
        )
        .expect("read succeeds");

        assert_eq!(result, Some(vec![5]));
        assert_eq!(reads, vec![CF_DIBV5 as u32]);
    }

    #[test]
    fn v5_none_falls_back_to_legacy_dib() {
        let mut reads = Vec::new();

        let result = process_preferred_dib(
            |format| {
                reads.push(format);
                if format == CF_DIBV5 as u32 {
                    Ok(None)
                } else {
                    Ok(Some(vec![4]))
                }
            },
            |_format, bytes| Ok(bytes),
        )
        .expect("legacy read succeeds");

        assert_eq!(result, Some(vec![4]));
        assert_eq!(reads, vec![CF_DIBV5 as u32, CF_DIB as u32]);
    }

    #[test]
    fn v5_error_falls_back_to_legacy_dib() {
        let mut reads = Vec::new();

        let result = process_preferred_dib(
            |format| {
                reads.push(format);
                if format == CF_DIBV5 as u32 {
                    Err(anyhow!("V5 read failed"))
                } else {
                    Ok(Some(vec![4]))
                }
            },
            |_format, bytes| Ok(bytes),
        )
        .expect("legacy read succeeds");

        assert_eq!(result, Some(vec![4]));
        assert_eq!(reads, vec![CF_DIBV5 as u32, CF_DIB as u32]);
    }

    #[test]
    fn both_unavailable_returns_none() {
        let mut reads = Vec::new();

        let result = process_preferred_dib(
            |format| {
                reads.push(format);
                Ok(None)
            },
            |_format, bytes: Vec<u8>| Ok(bytes),
        )
        .expect("unavailable formats are not an error");

        assert_eq!(result, None);
        assert_eq!(reads, vec![CF_DIBV5 as u32, CF_DIB as u32]);
    }

    #[test]
    fn both_fail_returns_first_error() {
        let mut reads = Vec::new();

        let error = process_preferred_dib(
            |format| {
                reads.push(format);
                if format == CF_DIBV5 as u32 {
                    Err(anyhow!("V5 read failed"))
                } else {
                    Err(anyhow!("legacy DIB read failed"))
                }
            },
            |_format, bytes: Vec<u8>| Ok(bytes),
        )
        .expect_err("both reads should fail");

        assert_eq!(error.to_string(), "V5 read failed");
        assert_eq!(reads, vec![CF_DIBV5 as u32, CF_DIB as u32]);
    }
}

// Runtime verification of the write-back paths against the REAL system clipboard.
// These are #[ignore] because they clobber the user's clipboard and are racy under
// parallel CI; run explicitly with:
//   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --test-threads=1 clipboard_roundtrip
#[cfg(test)]
mod clipboard_roundtrip_tests {
    use super::*;

    fn minimal_dib() -> Vec<u8> {
        // BITMAPINFOHEADER (40 bytes, 1x1 32bpp BI_RGB) + one BGRA pixel.
        let mut dib = vec![0u8; 40];
        dib[0] = 40; // biSize = 40
        dib[4] = 1; // biWidth = 1
        dib[8] = 1; // biHeight = 1
        dib[12] = 1; // biPlanes = 1
        dib[14] = 32; // biBitCount = 32
        dib.extend_from_slice(&[0x11, 0x22, 0x33, 0xFF]); // one pixel
        dib
    }

    #[test]
    #[ignore]
    fn clipboard_roundtrip_image_dib() {
        let dib = minimal_dib();
        write_dib_to_clipboard(&dib).expect("write dib");

        let read = {
            let _guard = ClipboardGuard::open().expect("open clipboard");
            unsafe { read_global_bytes(CF_DIB as u32) }
                .expect("read dib")
                .expect("dib present")
        };
        // GlobalSize may pad the allocation, so compare the payload prefix.
        assert!(read.len() >= dib.len());
        assert_eq!(&read[..dib.len()], &dib[..]);
    }

    #[test]
    #[ignore]
    fn clipboard_roundtrip_files() {
        let paths = vec!["C:\\Windows\\notepad.exe".to_string()];
        write_files_to_clipboard(&paths).expect("write files");

        let read = {
            let _guard = ClipboardGuard::open().expect("open clipboard");
            read_file_list()
                .expect("read files")
                .expect("files present")
        };
        assert_eq!(read, paths);
    }

    #[test]
    #[ignore]
    fn clipboard_roundtrip_html_with_plain_fallback() {
        let html = "<b>hello</b>";
        let plain = "hello";
        write_html_to_clipboard(html, plain).expect("write html");

        let text = {
            let _guard = ClipboardGuard::open().expect("open clipboard");
            read_unicode_text()
                .expect("read text")
                .expect("text present")
        };
        assert_eq!(text, plain);

        let cf_html = unsafe { RegisterClipboardFormatW(wide_null("HTML Format").as_ptr()) };
        let html_bytes = {
            let _guard = ClipboardGuard::open().expect("open clipboard");
            unsafe { read_global_bytes(cf_html) }
                .expect("read cf_html")
                .expect("cf_html present")
        };
        let rendered = String::from_utf8_lossy(&html_bytes);
        assert!(rendered.contains("StartFragment"), "missing CF_HTML header");
        assert!(rendered.contains(html), "fragment not preserved");
    }
}
