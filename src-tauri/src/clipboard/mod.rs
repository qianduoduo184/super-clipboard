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
use std::time::{Duration, Instant};

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

const CAPACITY_STATUS_MESSAGE: &str =
    "图片未保存：剪贴板图片存储空间已满，请取消收藏或删除历史图片。";

fn emit_capacity_status(app_handle: &AppHandle) {
    if let Err(error) = app_handle.emit("clipboard-status", CAPACITY_STATUS_MESSAGE) {
        crate::diagnostics::warn(format!(
            "clipboard: failed to emit capacity status event: {error}"
        ));
    }
}

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

#[cfg(test)]
fn store_image_capture(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    dib: Vec<u8>,
) -> anyhow::Result<ClipboardItem> {
    store_image_capture_with(repository, image_store, dib, || {})
}

#[cfg(test)]
fn store_image_capture_with(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    dib: Vec<u8>,
    on_existing: impl FnOnce(),
) -> anyhow::Result<ClipboardItem> {
    store_image_capture_with_hooks(repository, image_store, dib, on_existing, || {})
}

#[cfg(test)]
fn store_image_capture_with_hooks(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    dib: Vec<u8>,
    on_existing: impl FnOnce(),
    after_stage: impl FnOnce(),
) -> anyhow::Result<ClipboardItem> {
    store_image_capture_with_retention(repository, image_store, dib, 0, 0, on_existing, after_stage)
        .map(|(item, _)| item)
}

fn store_image_capture_with_retention(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    dib: Vec<u8>,
    max_history_items: i64,
    retention_days: i64,
    on_existing: impl FnOnce(),
    after_stage: impl FnOnce(),
) -> anyhow::Result<(ClipboardItem, bool)> {
    let bmp_size = crate::storage::capacity::exact_bmp_size(dib.len())?;
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
        return repository_guard
            .insert_or_touch_image(image_draft(
                &identity.content_hash,
                std::path::Path::new(content_path),
                size_bytes,
            ))
            .map(|item| (item, false));
    }
    drop(repository_guard);

    let reservation = crate::storage::capacity::capture_reservation(bmp_size)?;
    crate::storage::capacity::prune_for_capacity(
        repository,
        image_store,
        max_history_items,
        retention_days,
        reservation,
    )?;

    let staged = stage_dib(image_store.stage_dir(), dib)?;
    after_stage();
    anyhow::ensure!(
        staged.content_hash() == identity.content_hash,
        "staged image identity changed"
    );
    install_image_capture_outcome(repository, image_store, staged, size_bytes)
}

#[cfg(test)]
fn store_clipboard_capture(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    capture: ClipboardCapture,
    image_capture_enabled: bool,
) -> anyhow::Result<Option<ClipboardItem>> {
    store_clipboard_capture_with_retention(
        repository,
        image_store,
        capture,
        image_capture_enabled,
        0,
        0,
    )
    .map(|outcome| outcome.item)
}

struct StoredCaptureOutcome {
    item: Option<ClipboardItem>,
    created: bool,
    retention_pruned: bool,
}

fn store_clipboard_capture_with_retention(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    capture: ClipboardCapture,
    image_capture_enabled: bool,
    max_history_items: i64,
    retention_days: i64,
) -> anyhow::Result<StoredCaptureOutcome> {
    match capture {
        ClipboardCapture::Draft(draft) => {
            let repository = repository
                .lock()
                .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
            let created = repository.find_by_hash(&draft.stable_hash())?.is_none();
            let item = repository.insert_or_touch(draft)?;
            Ok(StoredCaptureOutcome {
                item: Some(item),
                created,
                retention_pruned: false,
            })
        }
        ClipboardCapture::ImageDib(_) if !image_capture_enabled => {
            crate::diagnostics::warn("clipboard: image capture blocked by storage quota");
            Ok(StoredCaptureOutcome {
                item: None,
                created: false,
                retention_pruned: false,
            })
        }
        ClipboardCapture::ImageDib(dib) => {
            let (item, created) = store_image_capture_with_retention(
                repository,
                image_store,
                dib,
                max_history_items,
                retention_days,
                || {},
                || {},
            )?;
            Ok(StoredCaptureOutcome {
                item: Some(item),
                created,
                retention_pruned: created,
            })
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

#[cfg(test)]
fn install_image_capture(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    staged: StagedImage,
    size_bytes: i64,
) -> anyhow::Result<ClipboardItem> {
    install_image_capture_outcome(repository, image_store, staged, size_bytes).map(|(item, _)| item)
}

fn install_image_capture_outcome(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    staged: StagedImage,
    size_bytes: i64,
) -> anyhow::Result<(ClipboardItem, bool)> {
    let content_hash = staged.content_hash().to_string();
    let result = image_store.install_staged_with_preflight(
        staged,
        |blob_dir, staged| {
            let repository_guard = repository
                .lock()
                .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
            if let Some(winner) = repository_guard.find_active_image(&content_hash)? {
                let winner_path = winner
                    .content_path
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("active image has no blob path"))?;
                let winner = repository_guard.insert_or_touch_image(image_draft(
                    &content_hash,
                    std::path::Path::new(winner_path),
                    size_bytes,
                ))?;
                return Err(ImageCaptureRace(winner).into());
            }
            drop(repository_guard);
            let additional = crate::storage::capacity::staged_allocation(
                staged.bmp_size(),
                staged.thumbnail_size(),
            )?;
            crate::storage::capacity::prune_for_capacity_locked(
                repository,
                blob_dir,
                0,
                0,
                additional,
                crate::storage::capacity::MANAGED_BLOB_QUOTA,
            )?;
            Ok(())
        },
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
        Ok(item) => Ok((item, true)),
        Err(error) => match error.downcast::<ImageCaptureRace>() {
            Ok(race) => Ok((race.0, false)),
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

    let listener_started = Instant::now();
    let prune_throttle = crate::storage::capacity::PruneThrottle::new_at(Duration::ZERO);
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
            let now = listener_started.elapsed();
            let prune_due = prune_throttle.is_due_at(now);
            let is_blocked_image =
                !image_capture_enabled && matches!(&capture, ClipboardCapture::ImageDib(_));
            let (max_history_items, retention_days) = if prune_due {
                (app_settings.max_history_items, app_settings.retention_days)
            } else {
                (0, 0)
            };
            let result = store_clipboard_capture_with_retention(
                &repository,
                &image_store,
                capture,
                image_capture_enabled,
                max_history_items,
                retention_days,
            );
            match result {
                Err(error) => {
                    if error
                        .downcast_ref::<crate::storage::capacity::CapacityError>()
                        .is_some()
                    {
                        emit_capacity_status(&app_handle);
                    }
                    crate::diagnostics::error(format!(
                        "clipboard: failed to store clipboard item: {error}"
                    ));
                }
                Ok(outcome) => {
                    if is_blocked_image {
                        emit_capacity_status(&app_handle);
                    }
                    if outcome.created && prune_due {
                        let prune_result = if outcome.retention_pruned {
                            Ok(())
                        } else {
                            crate::commands::prune_history_with(
                                &repository,
                                &image_store,
                                app_settings.max_history_items,
                                app_settings.retention_days,
                            )
                        };
                        if let Err(error) = prune_result {
                            crate::diagnostics::error(format!(
                                "clipboard: failed to prune clipboard history: {error}"
                            ));
                        } else {
                            prune_throttle.mark_pruned_at(now);
                        }
                    }
                    stored_any |= outcome.item.is_some();
                }
            }
        }

        if stored_any {
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
        store_image_capture_with, store_image_capture_with_hooks,
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
    fn oversized_duplicate_is_rejected_by_global_image_limit_without_mutation() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-oversized-duplicate-{}",
            Uuid::new_v4()
        ));
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let first = store_image_capture(&repository, &store, dib32(40, [30, 20, 10, 255], 0))
            .expect("first capture");
        let mut oversized = dib32(40, [30, 20, 10, 255], 0);
        oversized.resize(
            crate::storage::capacity::MAX_IMAGE_ALLOCATION as usize,
            0xAA,
        );

        let error = store_image_capture(&repository, &store, oversized)
            .expect_err("global image limit must apply before duplicate lookup");

        assert!(format!("{error:#}").contains("100 MiB"));
        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item(&first.id)
            .expect("first query")
            .is_some());
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
    fn second_capacity_check_removes_stage_and_leaves_history_unchanged() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-second-capacity-check-{}",
            Uuid::new_v4()
        ));
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let filler = store.blob_dir().join("concurrent-orphan.tmp");

        let error = store_image_capture_with_hooks(
            &repository,
            &store,
            dib32(40, [30, 20, 10, 255], 0),
            || {},
            || {
                let file = fs::File::create(&filler).expect("capacity filler");
                file.set_len(crate::storage::capacity::MANAGED_BLOB_QUOTA)
                    .expect("sparse capacity filler");
            },
        )
        .expect_err("second capacity check must reject concurrent usage");

        assert!(format!("{error:#}").contains("capacity"));
        assert!(repository
            .lock()
            .expect("repository lock")
            .active_blob_paths()
            .expect("active paths")
            .is_empty());
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
