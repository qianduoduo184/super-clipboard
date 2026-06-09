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
}

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::storage::repository::ClipboardRepository;
use crate::system::settings::AppSettings;

pub fn start_background_listener(
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

        let drafts = match win::read_current_clipboard(&blob_dir) {
            Ok(drafts) => drafts,
            Err(error) => {
                crate::diagnostics::error(format!("clipboard: failed to read clipboard: {error}"));
                return;
            }
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
                if let Err(error) = repository.prune_history(
                    app_settings.max_history_items,
                    app_settings.retention_days,
                ) {
                    crate::diagnostics::error(format!(
                        "clipboard: failed to prune clipboard history: {error}"
                    ));
                }
            }
        } else {
            crate::diagnostics::error("clipboard: repository lock poisoned");
        }
    })
}
