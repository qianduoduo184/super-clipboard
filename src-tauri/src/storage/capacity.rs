use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context};

use crate::blobs::store::ImageBlobStore;
use crate::storage::repository::ClipboardRepository;

pub const MAX_IMAGE_ALLOCATION: u64 = 100 * 1024 * 1024;
pub const MANAGED_BLOB_QUOTA: u64 = 5 * 1024 * 1024 * 1024;
pub const THUMBNAIL_RESERVATION: u64 = 1024 * 1024;
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct PruneThrottle {
    last_pruned_seconds: AtomicU64,
}

impl PruneThrottle {
    pub fn new_at(now: Duration) -> Self {
        Self {
            last_pruned_seconds: AtomicU64::new(now.as_secs()),
        }
    }

    pub fn is_due_at(&self, now: Duration) -> bool {
        now.as_secs()
            .saturating_sub(self.last_pruned_seconds.load(Ordering::Relaxed))
            >= PRUNE_INTERVAL.as_secs()
    }

    pub fn mark_pruned_at(&self, now: Duration) {
        self.last_pruned_seconds
            .store(now.as_secs(), Ordering::Relaxed);
    }
}

pub fn managed_usage(blob_dir: &Path) -> anyhow::Result<u64> {
    let metadata = fs::symlink_metadata(blob_dir)
        .with_context(|| format!("read managed blob directory {}", blob_dir.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && !is_reparse_point(&metadata),
        "managed blob root is not a regular directory: {}",
        blob_dir.display()
    );
    managed_usage_at(blob_dir)
}

pub(crate) fn exact_bmp_size(dib_len: usize) -> anyhow::Result<u64> {
    let dib_len = u64::try_from(dib_len).map_err(|_| anyhow!("DIB length does not fit in u64"))?;
    let bmp_size = 14_u64
        .checked_add(dib_len)
        .ok_or_else(|| anyhow!("BMP size overflow"))?;
    anyhow::ensure!(
        bmp_size <= MAX_IMAGE_ALLOCATION,
        "BMP file exceeds the 100 MiB image allocation limit"
    );
    Ok(bmp_size)
}

pub(crate) fn capture_reservation(bmp_size: u64) -> anyhow::Result<u64> {
    bmp_size
        .checked_add(THUMBNAIL_RESERVATION)
        .ok_or_else(|| anyhow!("image capacity reservation overflow"))
}

pub(crate) fn staged_allocation(bmp_size: u64, thumbnail_size: u64) -> anyhow::Result<u64> {
    bmp_size
        .checked_add(thumbnail_size)
        .ok_or_else(|| anyhow!("staged image allocation overflow"))
}

#[derive(Debug)]
pub struct CapacityError {
    usage: u64,
    additional: u64,
    quota: u64,
}

impl std::fmt::Display for CapacityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "managed blob capacity exceeded: usage={}, additional={}, quota={}; no non-favorite image remains eligible for automatic pruning",
            self.usage, self.additional, self.quota
        )
    }
}

impl std::error::Error for CapacityError {}

pub fn prune_for_capacity(
    repository: &Mutex<ClipboardRepository>,
    store: &ImageBlobStore,
    max_history_items: i64,
    retention_days: i64,
    additional: u64,
) -> anyhow::Result<u64> {
    prune_for_capacity_with_quota(
        repository,
        store,
        max_history_items,
        retention_days,
        additional,
        MANAGED_BLOB_QUOTA,
    )
}

fn prune_for_capacity_with_quota(
    repository: &Mutex<ClipboardRepository>,
    store: &ImageBlobStore,
    max_history_items: i64,
    retention_days: i64,
    additional: u64,
    quota: u64,
) -> anyhow::Result<u64> {
    store.with_write(|blob_dir, _| {
        prune_for_capacity_locked(
            repository,
            blob_dir,
            max_history_items,
            retention_days,
            additional,
            quota,
        )
    })
}

pub(crate) fn prune_for_capacity_locked(
    repository: &Mutex<ClipboardRepository>,
    blob_dir: &Path,
    max_history_items: i64,
    retention_days: i64,
    additional: u64,
    quota: u64,
) -> anyhow::Result<u64> {
    {
        let repository = repository
            .lock()
            .map_err(|error| anyhow!("repository lock poisoned: {error}"))?;
        repository.prune_history(max_history_items, retention_days)?;
    }
    crate::commands::cleanup_pending_blobs(repository, blob_dir)?;

    loop {
        let usage = managed_usage(blob_dir)?;
        let requested = usage
            .checked_add(additional)
            .ok_or_else(|| anyhow!("managed blob capacity arithmetic overflow"))?;
        if requested <= quota {
            return Ok(usage);
        }

        let candidate = repository
            .lock()
            .map_err(|error| anyhow!("repository lock poisoned: {error}"))?
            .oldest_capacity_candidate_id()?;
        let Some(candidate) = candidate else {
            return Err(CapacityError {
                usage,
                additional,
                quota,
            }
            .into());
        };
        {
            let repository = repository
                .lock()
                .map_err(|error| anyhow!("repository lock poisoned: {error}"))?;
            repository.soft_delete(&candidate)?;
        }
        crate::commands::cleanup_pending_blobs(repository, blob_dir)?;
    }
}

fn managed_usage_at(root: &Path) -> anyhow::Result<u64> {
    let mut usage = 0_u64;
    for entry in fs::read_dir(root)
        .with_context(|| format!("read managed blob directory {}", root.display()))?
    {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() || is_reparse_point(&metadata) {
            continue;
        }
        if file_type.is_dir() {
            usage = usage
                .checked_add(managed_usage_at(&entry.path())?)
                .ok_or_else(|| anyhow!("managed blob usage overflow"))?;
        } else if file_type.is_file() {
            usage = usage
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow!("managed blob usage overflow"))?;
        }
    }
    Ok(usage)
}

#[cfg(target_os = "windows")]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(target_os = "windows"))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use uuid::Uuid;

    use super::{
        capture_reservation, exact_bmp_size, managed_usage, prune_for_capacity_with_quota,
        PruneThrottle,
    };
    use crate::blobs::store::ImageBlobStore;
    use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};
    use crate::storage::repository::ClipboardRepository;

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "super-clipboard-capacity-{label}-{}",
            Uuid::new_v4()
        ))
    }

    #[test]
    fn managed_usage_counts_every_regular_file_below_blob_dir_only() {
        let root = temp_dir("recursive");
        let blob_dir = root.join("blobs");
        fs::create_dir_all(blob_dir.join("nested")).expect("blob tree");
        for (path, bytes) in [
            (blob_dir.join("canonical.bmp"), b"bmp".as_slice()),
            (blob_dir.join("canonical.thumb.png"), b"thumb".as_slice()),
            (blob_dir.join("orphan.tmp"), b"orphan".as_slice()),
            (blob_dir.join("cleanup-pending"), b"pending".as_slice()),
            (blob_dir.join("nested/deleted.bmp"), b"deleted".as_slice()),
        ] {
            fs::write(path, bytes).expect("managed file");
        }
        fs::write(root.join("history.sqlite3-wal"), b"wal").expect("database sibling");
        fs::write(root.join("backup.zip"), b"backup").expect("backup sibling");
        fs::write(root.join("import.stage"), b"stage").expect("import sibling");

        assert_eq!(
            managed_usage(&blob_dir).expect("managed usage"),
            3 + 5 + 6 + 7 + 7
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn managed_usage_reports_missing_root() {
        let root = temp_dir("missing");
        let error = managed_usage(&root).expect_err("missing blob directory must fail");
        assert!(format!("{error:#}").contains("managed blob directory"));
    }

    #[test]
    fn capture_reservation_is_exact_bmp_bytes_plus_thumbnail_reservation() {
        assert_eq!(
            capture_reservation(exact_bmp_size(44).expect("BMP size")).expect("reservation"),
            14 + 44 + 1024 * 1024
        );
        assert!(
            format!("{:#}", exact_bmp_size(100 * 1024 * 1024).unwrap_err()).contains("100 MiB")
        );
    }

    #[test]
    fn prune_throttle_becomes_due_at_ten_minutes_without_sleeping() {
        let throttle = PruneThrottle::new_at(std::time::Duration::ZERO);

        assert!(!throttle.is_due_at(std::time::Duration::ZERO));
        assert!(!throttle.is_due_at(std::time::Duration::from_secs(599)));
        assert!(throttle.is_due_at(std::time::Duration::from_secs(600)));
        throttle.mark_pruned_at(std::time::Duration::from_secs(600));
        assert!(!throttle.is_due_at(std::time::Duration::from_secs(600)));
    }

    fn insert_image(
        repository: &Mutex<ClipboardRepository>,
        path: &std::path::Path,
        content_hash: &str,
    ) -> crate::storage::repository::ClipboardItem {
        repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(path.to_string_lossy().to_string()),
                content_hash: Some(content_hash.to_string()),
                preview: "image".to_string(),
                source_app: None,
                size_bytes: fs::metadata(path).expect("image metadata").len() as i64,
            })
            .expect("insert image")
    }

    #[test]
    fn capacity_prune_never_deletes_favorites_and_returns_clear_error() {
        let root = temp_dir("favorite");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let favorite_path = store.blob_dir().join("favorite.bmp");
        fs::write(&favorite_path, b"favorite").expect("favorite blob");
        let favorite = insert_image(&repository, &favorite_path, "favorite-hash");
        repository
            .lock()
            .expect("repository lock")
            .toggle_favorite(&favorite.id)
            .expect("favorite");

        let error = prune_for_capacity_with_quota(&repository, &store, 10_000, 0, 0, 1)
            .expect_err("favorite-only usage cannot be reclaimed");

        assert!(format!("{error:#}").contains("capacity"));
        assert!(favorite_path.is_file());
        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item(&favorite.id)
            .expect("favorite query")
            .is_some());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn capacity_prune_deletes_pinned_non_favorite_before_favorite() {
        let root = temp_dir("pinned");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let favorite_path = store.blob_dir().join("favorite.bmp");
        let pinned_path = store.blob_dir().join("pinned.bmp");
        fs::write(&favorite_path, b"favorite").expect("favorite blob");
        fs::write(&pinned_path, b"pin").expect("pinned blob");
        let favorite = insert_image(&repository, &favorite_path, "favorite-order-hash");
        let pinned = insert_image(&repository, &pinned_path, "pinned-order-hash");
        {
            let repository = repository.lock().expect("repository lock");
            repository.toggle_favorite(&favorite.id).expect("favorite");
            repository.toggle_pin(&pinned.id).expect("pin");
        }

        assert_eq!(
            prune_for_capacity_with_quota(&repository, &store, 10_000, 0, 0, 8)
                .expect("pinned item is reclaimable"),
            8
        );

        assert!(favorite_path.is_file());
        assert!(!pinned_path.exists());
        let repository_guard = repository.lock().expect("repository lock");
        assert!(repository_guard
            .get_item(&favorite.id)
            .expect("favorite")
            .is_some());
        assert!(repository_guard
            .get_item(&pinned.id)
            .expect("pinned")
            .is_none());
        drop(repository_guard);
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn cleanup_rechecks_active_shared_paths_before_physical_delete() {
        let root = temp_dir("shared");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let shared_path = store.blob_dir().join("shared.bmp");
        fs::write(&shared_path, b"shared").expect("shared blob");
        insert_image(&repository, &shared_path, "shared-hash");
        repository
            .lock()
            .expect("repository lock")
            .update_image_references_and_enqueue_cleanup(&[], &[shared_path.clone()])
            .expect("enqueue shared cleanup");

        prune_for_capacity_with_quota(&repository, &store, 10_000, 0, 0, 100)
            .expect("active shared cleanup is skipped");

        assert!(shared_path.is_file());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn managed_usage_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("symlink");
        let blob_dir = root.join("blobs");
        let outside = root.join("outside");
        fs::create_dir_all(&blob_dir).expect("blob dir");
        fs::create_dir_all(&outside).expect("outside dir");
        fs::write(blob_dir.join("inside"), b"in").expect("inside file");
        fs::write(outside.join("large"), b"outside").expect("outside file");
        symlink(outside.join("large"), blob_dir.join("file-link")).expect("file symlink");
        symlink(&outside, blob_dir.join("dir-link")).expect("dir symlink");
        let root_link = root.with_extension("link");
        symlink(&outside, &root_link).expect("root symlink");

        assert_eq!(managed_usage(&blob_dir).expect("managed usage"), 2);
        assert!(managed_usage(&root_link).is_err());

        fs::remove_file(root_link).expect("root symlink cleanup");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
