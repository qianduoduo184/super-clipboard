pub mod types;

#[cfg(target_os = "windows")]
pub mod win;

#[cfg(not(target_os = "windows"))]
pub mod win {
    use anyhow::Result;
    use std::path::Path;

    use super::types::ClipboardItemDraft;

    pub fn start_listener<F>(_on_change: F) -> Result<()>
    where
        F: Fn() + Send + 'static,
    {
        Ok(())
    }

    pub fn read_current_clipboard(_blob_dir: &Path) -> Result<Vec<ClipboardItemDraft>> {
        Ok(Vec::new())
    }

    pub fn write_text_to_clipboard(_text: &str) -> Result<()> {
        Ok(())
    }

    pub fn write_html_to_clipboard(_html: &str, _plain_text: &str) -> Result<()> {
        Ok(())
    }

    pub fn write_dib_to_clipboard(_dib_bytes: &[u8]) -> Result<()> {
        Ok(())
    }

    pub fn write_files_to_clipboard(_file_paths: &[String]) -> Result<()> {
        Ok(())
    }

    pub fn simulate_paste_shortcut() -> Result<()> {
        Ok(())
    }
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::storage::repository::ClipboardRepository;
use crate::system::settings::AppSettings;
use tauri::{AppHandle, Emitter};

pub fn start_background_listener(
    app_handle: AppHandle,
    repository: Arc<Mutex<ClipboardRepository>>,
    settings: Arc<Mutex<AppSettings>>,
    blob_dir: PathBuf,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&blob_dir)?;
    crate::diagnostics::info(format!(
        "clipboard: preparing listener with blob_dir={}",
        blob_dir.display()
    ));

    win::start_listener(move || {
        crate::diagnostics::info("clipboard: change event received");
        let app_settings = settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default();

        if !app_settings.recording_enabled {
            crate::diagnostics::info("clipboard: recording disabled, event ignored");
            return;
        }

        let retry_delays = [
            Duration::from_millis(40),
            Duration::from_millis(80),
            Duration::from_millis(120),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ];
        let mut last_error = None;
        let mut drafts = None;
        for (attempt, delay) in retry_delays.iter().enumerate() {
            thread::sleep(*delay);
            match win::read_current_clipboard(&blob_dir) {
                Ok(next_drafts) => {
                    crate::diagnostics::info(format!(
                        "clipboard: read succeeded on attempt {}",
                        attempt + 1
                    ));
                    drafts = Some(next_drafts);
                    break;
                }
                Err(error) => {
                    crate::diagnostics::warn(format!(
                        "clipboard: read attempt {} failed: {error}",
                        attempt + 1
                    ));
                    last_error = Some(error);
                }
            }
        }
        let Some(drafts) = drafts else {
            crate::diagnostics::error(format!(
                "clipboard: failed to read clipboard after retries: {}",
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "unknown error".to_string())
            ));
            return;
        };
        crate::diagnostics::info(format!("clipboard: decoded {} item draft(s)", drafts.len()));

        if let Ok(repository) = repository.lock() {
            let mut stored_any = false;
            for draft in drafts {
                if let Err(error) = repository.insert_or_touch(draft) {
                    crate::diagnostics::error(format!(
                        "clipboard: failed to store clipboard item: {error}"
                    ));
                } else {
                    stored_any = true;
                }
            }

            if stored_any {
                if let Err(error) = repository
                    .prune_history(app_settings.max_history_items, app_settings.retention_days)
                {
                    crate::diagnostics::error(format!(
                        "clipboard: failed to prune clipboard history: {error}"
                    ));
                }
                if let Err(error) = app_handle.emit("clipboard-changed", ()) {
                    crate::diagnostics::warn(format!(
                        "clipboard: failed to emit change event: {error}"
                    ));
                }
            }
        } else {
            crate::diagnostics::error("clipboard: repository lock poisoned");
        }
    })
}
