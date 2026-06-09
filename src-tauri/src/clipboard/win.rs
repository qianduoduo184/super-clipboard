use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;

use anyhow::{anyhow, Result};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, IsClipboardFormatAvailable,
    EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_V,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, HWND_MESSAGE,
    MSG, WM_CLIPBOARDUPDATE, WM_DESTROY, WNDCLASSW,
};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};

use crate::blobs::build_blob_path;
use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};

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
            eprintln!("clipboard listener failed: {error}");
        }
    });

    Ok(())
}

pub fn read_current_clipboard(blob_dir: &Path) -> Result<Vec<ClipboardItemDraft>> {
    let _guard = ClipboardGuard::open()?;
    let mut drafts = Vec::new();

    if let Some(text) = read_unicode_text()? {
        if !text.trim().is_empty() {
            drafts.push(ClipboardItemDraft {
                item_type: ClipboardItemType::Text,
                size_bytes: text.len() as i64,
                preview: text.lines().next().unwrap_or_default().to_string(),
                content: Some(text),
                content_path: None,
                source_app: None,
            });
        }
    }

    if let Some(html) = read_html()? {
        drafts.push(ClipboardItemDraft {
            item_type: ClipboardItemType::Html,
            size_bytes: html.len() as i64,
            preview: html.lines().next().unwrap_or("<html>").to_string(),
            content: Some(html),
            content_path: None,
            source_app: None,
        });
    }

    if let Some(dib_bytes) = read_dib_bytes()? {
        let path = build_blob_path(blob_dir, "dib");
        std::fs::write(&path, &dib_bytes)?;
        drafts.push(ClipboardItemDraft {
            item_type: ClipboardItemType::Image,
            size_bytes: dib_bytes.len() as i64,
            preview: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("clipboard-image.dib")
                .to_string(),
            content: None,
            content_path: Some(path.to_string_lossy().to_string()),
            source_app: None,
        });
    }

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
        drafts.push(ClipboardItemDraft {
            item_type: ClipboardItemType::Files,
            size_bytes: files.len() as i64,
            preview,
            content: Some(serde_json::to_string(&files)?),
            content_path: None,
            source_app: None,
        });
    }

    Ok(drafts)
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

pub fn simulate_paste_shortcut() -> Result<()> {
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_V, false),
        keyboard_input(VK_V, true),
        keyboard_input(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(anyhow!("send Ctrl+V input"));
    }
    Ok(())
}

fn run_message_window() -> Result<()> {
    unsafe {
        let class_name = wide_null("SuperClipboardListenerWindow");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };

        if RegisterClassW(&window_class) == 0 {
            return Err(anyhow!("register clipboard listener window class"));
        }

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
        if hwnd == Default::default() {
            return Err(anyhow!("create clipboard listener window"));
        }

        if AddClipboardFormatListener(hwnd) == 0 {
            return Err(anyhow!("register clipboard listener"));
        }

        let mut message = MSG::default();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&message);
        }
    }

    Ok(())
}

extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
        WM_DESTROY => 0,
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Result<Self> {
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

fn read_html() -> Result<Option<String>> {
    unsafe {
        let html_format = RegisterClipboardFormatW(wide_null("HTML Format").as_ptr());
        if html_format == 0 || IsClipboardFormatAvailable(html_format) == 0 {
            return Ok(None);
        }

        read_global_bytes(html_format).map(|bytes| {
            bytes.map(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .trim_end_matches('\0')
                    .to_string()
            })
        })
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
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { Default::default() },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
