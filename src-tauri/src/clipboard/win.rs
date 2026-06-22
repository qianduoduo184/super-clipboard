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
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_V,
};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, HWND_MESSAGE, MSG, WM_CLIPBOARDUPDATE, WM_DESTROY, WNDCLASSW,
};

use crate::blobs::write_dib_as_bmp;
use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};

// DROPFILES structure for CF_HDROP
#[repr(C)]
#[allow(non_snake_case)]
struct DROPFILES {
    pFiles: u32,
    pt: POINT,
    fNC: i32,
    fWide: i32,
}

static CLIPBOARD_EVENT_TX: OnceLock<Mutex<Option<Sender<()>>>> = OnceLock::new();

pub fn start_listener<F>(on_change: F) -> Result<()>
where
    F: Fn() + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<()>();
    CLIPBOARD_EVENT_TX
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|error| anyhow!("clipboard event lock poisoned: {error}"))?
        .replace(tx);

    thread::spawn(move || {
        for _ in rx {
            on_change();
        }
    });

    thread::spawn(|| {
        if let Err(error) = run_message_window() {
            crate::diagnostics::error(format!("clipboard listener failed: {error}"));
        }
    });

    Ok(())
}

pub fn read_current_clipboard(blob_dir: &Path) -> Result<Vec<ClipboardItemDraft>> {
    let _guard = ClipboardGuard::open()?;

    if let Some(files) = read_file_list()? {
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
        return Ok(vec![ClipboardItemDraft {
            item_type: ClipboardItemType::Files,
            size_bytes: files.len() as i64,
            preview,
            content: Some(serde_json::to_string(&files)?),
            content_path: None,
            source_app: None,
        }]);
    }

    if let Some(dib_bytes) = read_dib_bytes()? {
        let path = write_dib_as_bmp(blob_dir, &dib_bytes)?;
        return Ok(vec![ClipboardItemDraft {
            item_type: ClipboardItemType::Image,
            size_bytes: dib_bytes.len() as i64,
            preview: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("clipboard-image.bmp")
                .to_string(),
            content: None,
            content_path: Some(path.to_string_lossy().to_string()),
            source_app: None,
        }]);
    }

    if let Some(text) = read_unicode_text()? {
        if !text.trim().is_empty() {
            // Compress all whitespace (newlines, tabs, multiple spaces) into single spaces for preview
            // The original multi-line content is preserved in 'content' field
            let preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
            return Ok(vec![ClipboardItemDraft {
                item_type: ClipboardItemType::Text,
                size_bytes: text.len() as i64,
                preview,
                content: Some(text),
                content_path: None,
                source_app: None,
            }]);
        }
    }

    Ok(Vec::new())
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

    Ok(())
}

pub fn simulate_paste_shortcut() -> Result<()> {
    // Wait for window focus to transition back to the previous application
    // after getCurrentWindow().hide() is called from the frontend.
    // Without this delay, SendInput may target the still-focused super-clipboard window.
    thread::sleep(Duration::from_millis(100));
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
            if let Some(lock) = CLIPBOARD_EVENT_TX.get() {
                if let Ok(guard) = lock.lock() {
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send(());
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
        while *locked.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(locked, len);
        let value = String::from_utf16_lossy(slice);
        GlobalUnlock(handle);
        Ok(Some(value))
    }
}

fn read_dib_bytes() -> Result<Option<Vec<u8>>> {
    unsafe {
        if IsClipboardFormatAvailable(CF_DIB as u32) == 0 {
            return Ok(None);
        }

        read_global_bytes(CF_DIB as u32)
    }
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
    let handle = GetClipboardData(format);
    if handle == Default::default() {
        return Err(anyhow!("get clipboard data"));
    }
    let size = GlobalSize(handle);
    if size == 0 {
        return Ok(None);
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
