pub mod sequence;
pub mod types;

#[cfg(target_os = "windows")]
pub mod win;

#[cfg(not(target_os = "windows"))]
pub mod win {
    use super::types::ClipboardCapture;
    use anyhow::Result;

    pub fn start_listener<F>(_on_change: F) -> Result<()>
    where
        F: Fn(u32) + Send + 'static,
    {
        Ok(())
    }

    pub fn read_current_clipboard(_event_sequence: u32) -> Result<Option<Vec<ClipboardCapture>>> {
        Ok(Some(Vec::new()))
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

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::storage::repository::ClipboardRepository;
use crate::system::settings::AppSettings;
use crate::{
    blobs::{
        image::{image_identity_from_dib, stage_dib, StagedImage},
        store::ImageBlobStore,
    },
    clipboard::types::{ClipboardCapture, ClipboardItemDraft, ClipboardItemType},
    storage::repository::ClipboardItem,
};
use tauri::{AppHandle, Emitter};

fn image_draft(
    content_hash: &str,
    content_path: &std::path::Path,
    size_bytes: i64,
) -> ClipboardItemDraft {
    ClipboardItemDraft {
        item_type: ClipboardItemType::Image,
        content: None,
        content_path: Some(content_path.to_string_lossy().to_string()),
        content_hash: Some(content_hash.to_string()),
        preview: content_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("clipboard-image.bmp")
            .to_string(),
        source_app: None,
        size_bytes,
    }
}

fn store_image_capture(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    dib: Vec<u8>,
) -> anyhow::Result<ClipboardItem> {
    store_image_capture_with(repository, image_store, dib, || {})
}

fn store_image_capture_with(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    dib: Vec<u8>,
    on_existing: impl FnOnce(),
) -> anyhow::Result<ClipboardItem> {
    let size_bytes = i64::try_from(dib.len()).unwrap_or(i64::MAX);
    let identity = image_identity_from_dib(&dib)?;
    let repository_guard = repository
        .lock()
        .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
    if let Some(existing) = repository_guard.find_active_image(&identity.content_hash)? {
        on_existing();
        let content_path = existing
            .content_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("active image has no blob path"))?;
        return repository_guard.insert_or_touch_image(image_draft(
            &identity.content_hash,
            std::path::Path::new(content_path),
            size_bytes,
        ));
    }
    drop(repository_guard);

    let staged = stage_dib(image_store.stage_dir(), dib)?;
    anyhow::ensure!(
        staged.content_hash() == identity.content_hash,
        "staged image identity changed"
    );
    install_image_capture(repository, image_store, staged, size_bytes)
}

fn store_clipboard_capture(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    capture: ClipboardCapture,
    image_capture_enabled: bool,
) -> anyhow::Result<Option<ClipboardItem>> {
    match capture {
        ClipboardCapture::Draft(draft) => repository
            .lock()
            .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
            .insert_or_touch(draft)
            .map(Some),
        ClipboardCapture::ImageDib(_) if !image_capture_enabled => {
            crate::diagnostics::warn("clipboard: image capture blocked by storage quota");
            Ok(None)
        }
        ClipboardCapture::ImageDib(dib) => {
            store_image_capture(repository, image_store, dib).map(Some)
        }
    }
}

#[derive(Debug)]
struct ImageCaptureRace(ClipboardItem);

impl std::fmt::Display for ImageCaptureRace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("active image appeared while installing capture")
    }
}

impl std::error::Error for ImageCaptureRace {}

fn install_image_capture(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    staged: StagedImage,
    size_bytes: i64,
) -> anyhow::Result<ClipboardItem> {
    let content_hash = staged.content_hash().to_string();
    let result = image_store.install_staged_with(
        staged,
        |installed| {
            let repository = repository
                .lock()
                .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
            if let Some(winner) = repository.find_active_image(&content_hash)? {
                let winner_path = winner
                    .content_path
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("active image has no blob path"))?;
                let winner = repository.insert_or_touch_image(image_draft(
                    &content_hash,
                    std::path::Path::new(winner_path),
                    size_bytes,
                ))?;
                return Err(ImageCaptureRace(winner).into());
            }
            repository.insert_or_touch_image(image_draft(
                installed.content_hash(),
                installed.bmp_path(),
                size_bytes,
            ))
        },
        |installed| {
            Ok(repository
                .lock()
                .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
                .active_blob_paths()?
                .iter()
                .any(|path| path == installed.bmp_path()))
        },
    );
    match result {
        Ok(item) => Ok(item),
        Err(error) => match error.downcast::<ImageCaptureRace>() {
            Ok(race) => Ok(race.0),
            Err(error) => Err(error),
        },
    }
}

pub fn start_background_listener(
    app_handle: AppHandle,
    repository: Arc<Mutex<ClipboardRepository>>,
    settings: Arc<Mutex<AppSettings>>,
    image_store: Arc<ImageBlobStore>,
    image_capture_enabled: bool,
) -> anyhow::Result<()> {
    crate::diagnostics::info(format!(
        "clipboard: preparing listener with blob_dir={}",
        image_store.blob_dir().display()
    ));

    win::start_listener(move |event_sequence| {
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
            match win::read_current_clipboard(event_sequence) {
                Ok(Some(next_drafts)) => {
                    crate::diagnostics::info(format!(
                        "clipboard: read succeeded on attempt {}",
                        attempt + 1
                    ));
                    drafts = Some(next_drafts);
                    break;
                }
                Ok(None) => {
                    crate::diagnostics::info(format!(
                        "clipboard: stale sequence {event_sequence} ignored"
                    ));
                    return;
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

        let mut stored_any = false;
        for capture in drafts {
            let result =
                store_clipboard_capture(&repository, &image_store, capture, image_capture_enabled);
            match result {
                Err(error) => {
                    crate::diagnostics::error(format!(
                        "clipboard: failed to store clipboard item: {error}"
                    ));
                }
                Ok(stored) => {
                    stored_any |= stored.is_some();
                }
            }
        }

        if stored_any {
            if let Err(error) = crate::commands::prune_history_with(
                &repository,
                &image_store,
                app_settings.max_history_items,
                app_settings.retention_days,
            ) {
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
    })
}

#[cfg(test)]
mod tests {
    use super::{
        image_draft, install_image_capture, store_clipboard_capture, store_image_capture,
        store_image_capture_with,
    };
    use crate::blobs::image::stage_dib;
    use crate::blobs::store::ImageBlobStore;
    use crate::storage::repository::ClipboardRepository;
    use std::fs;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use uuid::Uuid;

    fn dib32(header_size: u32, pixel: [u8; 4], trailing_padding: usize) -> Vec<u8> {
        let mut dib = vec![0u8; header_size as usize];
        dib[0..4].copy_from_slice(&header_size.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[20..24].copy_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&pixel);
        dib.resize(dib.len() + trailing_padding, 0xAA);
        dib
    }

    #[test]
    fn quota_gate_blocks_image_capture_without_writing_rows_or_blobs() {
        let root =
            std::env::temp_dir().join(format!("super-clipboard-quota-gate-{}", Uuid::new_v4()));
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");

        let stored = store_clipboard_capture(
            &repository,
            &store,
            crate::clipboard::types::ClipboardCapture::ImageDib(dib32(40, [30, 20, 10, 255], 0)),
            false,
        )
        .expect("blocked capture");

        assert!(stored.is_none());
        assert!(repository
            .lock()
            .expect("repository lock")
            .active_blob_paths()
            .expect("active paths")
            .is_empty());
        assert!(fs::read_dir(store.blob_dir())
            .expect("blob directory")
            .next()
            .is_none());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn duplicate_image_capture_touches_canonical_item_without_creating_files() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-duplicate-image-{}",
            Uuid::new_v4()
        ));
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");

        let first = store_image_capture(&repository, &store, dib32(40, [30, 20, 10, 255], 0))
            .expect("first capture");
        let second = store_image_capture(&repository, &store, dib32(124, [30, 20, 10, 255], 64))
            .expect("duplicate capture");

        assert_eq!(first.id, second.id);
        assert_eq!(
            fs::read_dir(store.blob_dir())
                .expect("blob directory")
                .count(),
            2
        );
        assert!(fs::read_dir(store.stage_dir())
            .expect("stage directory")
            .next()
            .is_none());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn image_capture_race_rolls_back_only_unreferenced_created_files() {
        let root =
            std::env::temp_dir().join(format!("super-clipboard-image-race-{}", Uuid::new_v4()));
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let staged =
            stage_dib(store.stage_dir(), dib32(40, [30, 20, 10, 255], 0)).expect("stage loser");
        let winner = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(image_draft(
                staged.content_hash(),
                &root.join("legacy-winner.bmp"),
                44,
            ))
            .expect("insert winner");

        let captured = install_image_capture(&repository, &store, staged, 44)
            .expect("resolve image capture race");

        assert_eq!(captured.id, winner.id);
        assert!(fs::read_dir(store.blob_dir())
            .expect("blob directory")
            .next()
            .is_none());
        assert!(fs::read_dir(store.stage_dir())
            .expect("stage directory")
            .next()
            .is_none());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn duplicate_capture_does_not_rebuild_row_after_concurrent_delete() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-capture-delete-race-{}",
            Uuid::new_v4()
        ));
        let repository = Arc::new(Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        ));
        let store =
            Arc::new(ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store"));
        let dib = dib32(40, [30, 20, 10, 255], 0);
        let first = store_image_capture(&repository, &store, dib.clone()).expect("first capture");
        let content_hash = first.content_hash.clone().expect("content hash");
        let (delete_start_tx, delete_start_rx) = mpsc::channel();
        let (delete_done_tx, delete_done_rx) = mpsc::channel();
        let delete_repository = Arc::clone(&repository);
        let delete_store = Arc::clone(&store);
        let first_id = first.id.clone();
        let delete_worker = thread::spawn(move || {
            delete_start_rx.recv().expect("delete start");
            crate::commands::mutate_and_cleanup_blobs(
                &delete_repository,
                &delete_store,
                |repository| repository.soft_delete(&first_id),
            )
            .expect("delete image");
            delete_done_tx.send(()).expect("delete done");
        });

        store_image_capture_with(&repository, &store, dib, || {
            delete_start_tx.send(()).expect("start delete");
            thread::sleep(Duration::from_millis(100));
        })
        .expect("duplicate capture");
        delete_done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("delete completed");
        delete_worker.join().expect("delete worker");

        assert!(repository
            .lock()
            .expect("repository lock")
            .find_active_image(&content_hash)
            .expect("active image")
            .is_none());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
