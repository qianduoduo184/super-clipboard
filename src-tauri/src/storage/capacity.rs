use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::blobs::store::ImageBlobStore;
use crate::storage::repository::{ClipboardItem, ClipboardRepository};

pub const MAX_IMAGE_ALLOCATION: u64 = 100 * 1024 * 1024;
pub const MANAGED_BLOB_QUOTA: u64 = 5 * 1024 * 1024 * 1024;
pub const THUMBNAIL_RESERVATION: u64 = 1024 * 1024;
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(600);
pub const CAPACITY_STATUS_MESSAGE: &str =
    "图片未保存：剪贴板图片存储空间已满，请取消收藏或删除历史图片。";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardCapacityStatus {
    pub blocked: bool,
    pub message: String,
    pub required_additional: u64,
    pub revision: u64,
}

pub fn current_capacity_status(
    state: &Mutex<ClipboardCapacityStatus>,
) -> anyhow::Result<ClipboardCapacityStatus> {
    state
        .lock()
        .map(|status| status.clone())
        .map_err(|error| anyhow!("clipboard capacity status lock poisoned: {error}"))
}

pub fn update_capacity_status_with(
    state: &Mutex<ClipboardCapacityStatus>,
    blocked: bool,
    required_additional: u64,
    emit: impl FnOnce(&ClipboardCapacityStatus) -> anyhow::Result<()>,
) -> anyhow::Result<ClipboardCapacityStatus> {
    let required_additional = if blocked { required_additional } else { 0 };
    let message = if blocked {
        CAPACITY_STATUS_MESSAGE.to_string()
    } else {
        String::new()
    };
    let next = {
        let mut current = state
            .lock()
            .map_err(|error| anyhow!("clipboard capacity status lock poisoned: {error}"))?;
        if current.blocked != blocked
            || current.message != message
            || current.required_additional != required_additional
        {
            let revision = current
                .revision
                .checked_add(1)
                .ok_or_else(|| anyhow!("clipboard capacity status revision overflow"))?;
            *current = ClipboardCapacityStatus {
                blocked,
                message,
                required_additional,
                revision,
            };
        }
        current.clone()
    };
    emit(&next)?;
    Ok(next)
}

pub fn publish_capacity_status(
    app_handle: &tauri::AppHandle,
    state: &Mutex<ClipboardCapacityStatus>,
    blocked: bool,
    required_additional: u64,
) -> anyhow::Result<ClipboardCapacityStatus> {
    use tauri::Emitter;

    update_capacity_status_with(state, blocked, required_additional, |status| {
        app_handle
            .emit("clipboard-status", status)
            .map_err(Into::into)
    })
}

pub fn clear_capacity_status_if_recovered(
    app_handle: &tauri::AppHandle,
    state: &Mutex<ClipboardCapacityStatus>,
    image_store: &ImageBlobStore,
) -> anyhow::Result<ClipboardCapacityStatus> {
    use tauri::Emitter;

    clear_capacity_status_if_recovered_with(state, image_store, |status| {
        app_handle
            .emit("clipboard-status", status)
            .map_err(Into::into)
    })
}

pub(crate) fn clear_capacity_status_if_recovered_with(
    state: &Mutex<ClipboardCapacityStatus>,
    image_store: &ImageBlobStore,
    emit: impl FnOnce(&ClipboardCapacityStatus) -> anyhow::Result<()>,
) -> anyhow::Result<ClipboardCapacityStatus> {
    clear_capacity_status_if_recovered_using(state, image_store, managed_usage, emit)
}

fn clear_capacity_status_if_recovered_using(
    state: &Mutex<ClipboardCapacityStatus>,
    image_store: &ImageBlobStore,
    measure_usage: impl FnOnce(&Path) -> anyhow::Result<u64>,
    emit: impl FnOnce(&ClipboardCapacityStatus) -> anyhow::Result<()>,
) -> anyhow::Result<ClipboardCapacityStatus> {
    image_store.with_read(|blob_dir| {
        let current = current_capacity_status(state)?;
        if !current.blocked {
            return republish_capacity_status_if_unchanged_with(state, &current, emit);
        }
        let usage = measure_usage(blob_dir)?;
        if capacity_fits(usage, current.required_additional, MANAGED_BLOB_QUOTA)? {
            clear_capacity_status_if_unchanged_with(state, &current, emit)
        } else {
            Ok(current)
        }
    })
}

fn republish_capacity_status_if_unchanged_with(
    state: &Mutex<ClipboardCapacityStatus>,
    expected: &ClipboardCapacityStatus,
    emit: impl FnOnce(&ClipboardCapacityStatus) -> anyhow::Result<()>,
) -> anyhow::Result<ClipboardCapacityStatus> {
    let current = current_capacity_status(state)?;
    if current != *expected {
        return Ok(current);
    }
    emit(&current)?;
    Ok(current)
}

fn clear_capacity_status_if_unchanged_with(
    state: &Mutex<ClipboardCapacityStatus>,
    expected: &ClipboardCapacityStatus,
    emit: impl FnOnce(&ClipboardCapacityStatus) -> anyhow::Result<()>,
) -> anyhow::Result<ClipboardCapacityStatus> {
    let next = {
        let mut current = state
            .lock()
            .map_err(|error| anyhow!("clipboard capacity status lock poisoned: {error}"))?;
        if !current.blocked
            || current.revision != expected.revision
            || current.required_additional != expected.required_additional
        {
            return Ok(current.clone());
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow!("clipboard capacity status revision overflow"))?;
        *current = ClipboardCapacityStatus {
            blocked: false,
            message: String::new(),
            required_additional: 0,
            revision,
        };
        current.clone()
    };
    emit(&next)?;
    Ok(next)
}

pub(crate) fn clear_capacity_status_after_image_capture_with(
    state: &Mutex<ClipboardCapacityStatus>,
    image_store: &ImageBlobStore,
    created: bool,
    emit: impl FnOnce(&ClipboardCapacityStatus) -> anyhow::Result<()>,
) -> anyhow::Result<ClipboardCapacityStatus> {
    if created {
        clear_capacity_status_if_recovered_with(state, image_store, emit)
    } else {
        current_capacity_status(state)
    }
}

pub fn clear_capacity_status_after_image_capture(
    app_handle: &tauri::AppHandle,
    state: &Mutex<ClipboardCapacityStatus>,
    image_store: &ImageBlobStore,
    created: bool,
) -> anyhow::Result<ClipboardCapacityStatus> {
    use tauri::Emitter;

    clear_capacity_status_after_image_capture_with(state, image_store, created, |status| {
        app_handle
            .emit("clipboard-status", status)
            .map_err(Into::into)
    })
}

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

impl CapacityError {
    pub fn required_additional(&self) -> u64 {
        self.additional
    }
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
    crate::commands::cleanup_pending_blobs(repository, blob_dir)?;
    let items = {
        let repository = repository
            .lock()
            .map_err(|error| anyhow!("repository lock poisoned: {error}"))?;
        repository.capacity_items()?
    };
    let usage = managed_usage(blob_dir)?;
    let plan = build_capacity_prune_plan(
        &items,
        blob_dir,
        usage,
        max_history_items,
        retention_days,
        Utc::now().timestamp_micros(),
        additional,
        quota,
    )?;
    if !plan.candidate_ids.is_empty() {
        {
            let repository = repository
                .lock()
                .map_err(|error| anyhow!("repository lock poisoned: {error}"))?;
            repository.soft_delete_batch(&plan.candidate_ids)?;
        }
    }
    crate::commands::cleanup_pending_blobs(repository, blob_dir)?;
    let usage = managed_usage(blob_dir)?;
    ensure_capacity(usage, additional, quota)?;
    Ok(usage)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapacityPrunePlan {
    candidate_ids: Vec<String>,
    projected_usage: u64,
}

fn build_capacity_prune_plan(
    items: &[ClipboardItem],
    blob_dir: &Path,
    usage: u64,
    max_history_items: i64,
    retention_days: i64,
    now: i64,
    additional: u64,
    quota: u64,
) -> anyhow::Result<CapacityPrunePlan> {
    let mut active_refs = HashMap::<std::path::PathBuf, usize>::new();
    for item in items {
        if item.item_type == "image" {
            if let Some(path) = item.content_path.as_deref() {
                *active_refs.entry(path.into()).or_default() += 1;
            }
        }
    }
    let mut selected_refs = HashMap::<std::path::PathBuf, usize>::new();
    let mut selected_ids = HashSet::<String>::new();
    let mut candidate_ids = Vec::new();
    let mut reclaimable = 0_u64;

    let mut age_candidates = Vec::new();
    if retention_days > 0 {
        let retention_micros = retention_days.saturating_mul(24 * 60 * 60 * 1_000_000);
        let cutoff = now.saturating_sub(retention_micros);
        age_candidates.extend(
            items
                .iter()
                .filter(|item| !item.favorite && item.updated_at < cutoff),
        );
        sort_oldest_first(&mut age_candidates);
    }
    for item in age_candidates {
        select_candidate(
            item,
            blob_dir,
            &active_refs,
            &mut selected_refs,
            &mut selected_ids,
            &mut candidate_ids,
            &mut reclaimable,
        )?;
    }

    if max_history_items > 0 {
        let mut remaining = items
            .iter()
            .filter(|item| !item.favorite && !selected_ids.contains(&item.id))
            .collect::<Vec<_>>();
        remaining.sort_by(|left, right| newest_order(left, right));
        let keep = usize::try_from(max_history_items).unwrap_or(usize::MAX);
        let mut count_candidates = remaining.into_iter().skip(keep).collect::<Vec<_>>();
        sort_oldest_first(&mut count_candidates);
        for item in count_candidates {
            select_candidate(
                item,
                blob_dir,
                &active_refs,
                &mut selected_refs,
                &mut selected_ids,
                &mut candidate_ids,
                &mut reclaimable,
            )?;
        }
    }

    let mut projected_usage = usage.saturating_sub(reclaimable);
    if !capacity_fits(projected_usage, additional, quota)? {
        let mut byte_candidates = items
            .iter()
            .filter(|item| {
                item.item_type == "image" && !item.favorite && !selected_ids.contains(&item.id)
            })
            .collect::<Vec<_>>();
        sort_oldest_first(&mut byte_candidates);
        for item in byte_candidates {
            select_candidate(
                item,
                blob_dir,
                &active_refs,
                &mut selected_refs,
                &mut selected_ids,
                &mut candidate_ids,
                &mut reclaimable,
            )?;
            projected_usage = usage.saturating_sub(reclaimable);
            if capacity_fits(projected_usage, additional, quota)? {
                break;
            }
        }
    }

    projected_usage = usage.saturating_sub(reclaimable);
    if !capacity_fits(projected_usage, additional, quota)? {
        return Err(CapacityError {
            usage,
            additional,
            quota,
        }
        .into());
    }
    Ok(CapacityPrunePlan {
        candidate_ids,
        projected_usage,
    })
}

fn select_candidate(
    item: &ClipboardItem,
    blob_dir: &Path,
    active_refs: &HashMap<std::path::PathBuf, usize>,
    selected_refs: &mut HashMap<std::path::PathBuf, usize>,
    selected_ids: &mut HashSet<String>,
    candidate_ids: &mut Vec<String>,
    reclaimable: &mut u64,
) -> anyhow::Result<()> {
    if !selected_ids.insert(item.id.clone()) {
        return Ok(());
    }
    candidate_ids.push(item.id.clone());
    if item.item_type != "image" {
        return Ok(());
    }
    let Some(path) = item.content_path.as_deref().map(std::path::PathBuf::from) else {
        return Ok(());
    };
    let selected = selected_refs.entry(path.clone()).or_default();
    *selected += 1;
    if active_refs.get(&path).copied() == Some(*selected) {
        *reclaimable = reclaimable
            .checked_add(reclaimable_image_bytes(blob_dir, &path)?)
            .ok_or_else(|| anyhow!("capacity reclaimable byte count overflow"))?;
    }
    Ok(())
}

fn reclaimable_image_bytes(blob_dir: &Path, bmp_path: &Path) -> anyhow::Result<u64> {
    let mut bytes = 0_u64;
    for path in [
        bmp_path.to_path_buf(),
        crate::blobs::thumbnail_path_for(bmp_path),
    ] {
        if !path.is_absolute() || path.parent() != Some(blob_dir) {
            continue;
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file()
                    && !metadata.file_type().is_symlink()
                    && !is_reparse_point(&metadata) =>
            {
                bytes = bytes
                    .checked_add(metadata.len())
                    .ok_or_else(|| anyhow!("capacity reclaimable byte count overflow"))?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(bytes)
}

fn sort_oldest_first(items: &mut Vec<&ClipboardItem>) {
    items.sort_by(|left, right| {
        left.updated_at
            .cmp(&right.updated_at)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn newest_order(left: &ClipboardItem, right: &ClipboardItem) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| right.created_at.cmp(&left.created_at))
        .then_with(|| right.id.cmp(&left.id))
}

fn capacity_fits(usage: u64, additional: u64, quota: u64) -> anyhow::Result<bool> {
    Ok(usage
        .checked_add(additional)
        .ok_or_else(|| anyhow!("managed blob capacity arithmetic overflow"))?
        <= quota)
}

fn ensure_capacity(usage: u64, additional: u64, quota: u64) -> anyhow::Result<()> {
    if capacity_fits(usage, additional, quota)? {
        Ok(())
    } else {
        Err(CapacityError {
            usage,
            additional,
            quota,
        }
        .into())
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
        build_capacity_prune_plan, capture_reservation,
        clear_capacity_status_after_image_capture_with, clear_capacity_status_if_recovered_using,
        clear_capacity_status_if_recovered_with, current_capacity_status, exact_bmp_size,
        managed_usage, prune_for_capacity_with_quota, republish_capacity_status_if_unchanged_with,
        update_capacity_status_with, ClipboardCapacityStatus, PruneThrottle, MANAGED_BLOB_QUOTA,
    };
    use crate::blobs::store::ImageBlobStore;
    use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};
    use crate::storage::repository::{ClipboardItem, ClipboardRepository};

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

    fn planner_item(id: &str, path: &std::path::Path, updated_at: i64) -> ClipboardItem {
        ClipboardItem {
            id: id.to_string(),
            hash: format!("image:{id}"),
            item_type: "image".to_string(),
            content: None,
            content_path: Some(path.to_string_lossy().to_string()),
            content_hash: Some(id.to_string()),
            preview: id.to_string(),
            source_app: None,
            favorite: false,
            pinned: false,
            size_bytes: 2,
            created_at: updated_at,
            updated_at,
        }
    }

    #[test]
    fn planner_orders_age_then_count_then_oldest_byte_pressure() {
        const DAY_MICROS: i64 = 24 * 60 * 60 * 1_000_000;
        let root = temp_dir("planner-order");
        let blob_dir = root.join("blobs");
        fs::create_dir_all(&blob_dir).expect("blob dir");
        let now = 2 * DAY_MICROS;
        let cutoff = now - DAY_MICROS;
        let definitions = [
            ("age", 0),
            ("count", cutoff + 10),
            ("byte", cutoff + 20),
            ("kept-middle", cutoff + 30),
            ("kept-newest", cutoff + 40),
        ];
        let mut items = Vec::new();
        for (id, updated_at) in definitions {
            let path = blob_dir.join(format!("{id}.bmp"));
            fs::write(&path, b"xx").expect("planner blob");
            items.push(planner_item(id, &path, updated_at));
        }

        let plan = build_capacity_prune_plan(&items, &blob_dir, 10, 3, 1, now, 0, 4)
            .expect("capacity plan");

        assert_eq!(plan.candidate_ids, vec!["age", "count", "byte"]);
        assert_eq!(plan.projected_usage, 4);
        fs::remove_dir_all(root).expect("cleanup");
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
    fn insufficient_capacity_does_not_partially_delete_eligible_history() {
        let root = temp_dir("atomic-failure");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let first_path = store.blob_dir().join("first.bmp");
        let second_path = store.blob_dir().join("second.bmp");
        let favorite_path = store.blob_dir().join("favorite.bmp");
        let orphan_path = store.blob_dir().join("orphan.tmp");
        fs::write(&first_path, b"one").expect("first blob");
        fs::write(&second_path, b"two").expect("second blob");
        fs::write(&favorite_path, b"favorite").expect("favorite blob");
        fs::write(&orphan_path, b"orphan").expect("orphan blob");
        let first = insert_image(&repository, &first_path, "atomic-first");
        let second = insert_image(&repository, &second_path, "atomic-second");
        let favorite = insert_image(&repository, &favorite_path, "atomic-favorite");
        repository
            .lock()
            .expect("repository lock")
            .toggle_favorite(&favorite.id)
            .expect("favorite");

        let error = prune_for_capacity_with_quota(&repository, &store, 10_000, 0, 0, 4)
            .expect_err("eligible bytes are insufficient");

        assert!(error.downcast_ref::<super::CapacityError>().is_some());
        let repository_guard = repository.lock().expect("repository lock");
        for id in [&first.id, &second.id, &favorite.id] {
            assert!(repository_guard
                .get_item(id)
                .expect("history query")
                .is_some());
        }
        assert!(repository_guard
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository_guard);
        for path in [&first_path, &second_path, &favorite_path, &orphan_path] {
            assert!(path.is_file(), "missing {}", path.display());
        }
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

    #[test]
    fn pending_cleanup_is_recovered_before_capacity_planning() {
        let root = temp_dir("pending-first");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let pending_bmp = store.blob_dir().join("pending.bmp");
        let pending_thumbnail = crate::blobs::thumbnail_path_for(&pending_bmp);
        let absent = store.blob_dir().join("absent.bmp");
        fs::write(&pending_bmp, b"12345").expect("pending bmp");
        fs::write(&pending_thumbnail, b"67890").expect("pending thumbnail");
        repository
            .lock()
            .expect("repository lock")
            .update_image_references_and_enqueue_cleanup(&[], &[pending_bmp.clone(), absent])
            .expect("enqueue pending cleanup");

        let usage = prune_for_capacity_with_quota(&repository, &store, 0, 0, 6, 6)
            .expect("pending cleanup should restore capacity first");

        assert_eq!(usage, 0);
        assert!(!pending_bmp.exists());
        assert!(!pending_thumbnail.exists());
        assert!(repository
            .lock()
            .expect("repository lock")
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn capacity_status_persists_emits_and_clears_by_revision() {
        let state = Mutex::new(ClipboardCapacityStatus::default());
        let emitted = std::cell::RefCell::new(Vec::new());

        let blocked = update_capacity_status_with(&state, true, 7, |status| {
            emitted.borrow_mut().push(status.clone());
            Ok(())
        })
        .expect("blocked status");

        assert!(blocked.blocked);
        assert!(!blocked.message.is_empty());
        assert_eq!(blocked.required_additional, 7);
        assert_eq!(blocked.revision, 1);
        assert_eq!(current_capacity_status(&state).expect("query"), blocked);
        assert_eq!(&*emitted.borrow(), &[blocked.clone()]);

        let unchanged = update_capacity_status_with(&state, true, 7, |status| {
            emitted.borrow_mut().push(status.clone());
            Ok(())
        })
        .expect("unchanged status");
        assert_eq!(unchanged.revision, 1);
        assert_eq!(emitted.borrow().len(), 2);

        let changed_requirement = update_capacity_status_with(&state, true, 8, |status| {
            emitted.borrow_mut().push(status.clone());
            Ok(())
        })
        .expect("changed requirement");
        assert_eq!(changed_requirement.required_additional, 8);
        assert_eq!(changed_requirement.revision, 2);

        let cleared = update_capacity_status_with(&state, false, 0, |status| {
            emitted.borrow_mut().push(status.clone());
            Ok(())
        })
        .expect("clear status");

        assert!(!cleared.blocked);
        assert!(cleared.message.is_empty());
        assert_eq!(cleared.required_additional, 0);
        assert_eq!(cleared.revision, 3);
        assert_eq!(
            current_capacity_status(&state).expect("query clear"),
            cleared
        );
        assert_eq!(emitted.borrow().len(), 4);
    }

    #[test]
    fn unchanged_capacity_status_retries_emit_after_sink_failure() {
        let state = Mutex::new(ClipboardCapacityStatus::default());
        let calls = std::cell::Cell::new(0);

        update_capacity_status_with(&state, true, 13, |_| {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("injected emit failure"))
        })
        .expect_err("first emit must fail");
        let persisted = current_capacity_status(&state).expect("persisted after failure");
        assert!(persisted.blocked);
        assert_eq!(persisted.required_additional, 13);
        assert_eq!(persisted.revision, 1);

        let retried = update_capacity_status_with(&state, true, 13, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .expect("retry same status");

        assert_eq!(calls.get(), 2);
        assert_eq!(retried.revision, 1);
        assert_eq!(
            current_capacity_status(&state).expect("query retry"),
            retried
        );
    }

    #[test]
    fn recovery_retries_unchanged_clear_after_sink_failure() {
        let root = temp_dir("status-clear-retry");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let state = Mutex::new(ClipboardCapacityStatus::default());
        update_capacity_status_with(&state, true, 1, |_| Ok(())).expect("blocked status");
        let calls = std::cell::Cell::new(0);

        clear_capacity_status_if_recovered_with(&state, &store, |_| {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("injected clear emit failure"))
        })
        .expect_err("first clear emit must fail");
        let persisted = current_capacity_status(&state).expect("persisted clear");
        assert!(!persisted.blocked);
        assert_eq!(persisted.revision, 2);

        let retried = clear_capacity_status_if_recovered_with(&state, &store, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .expect("retry clear");

        assert_eq!(calls.get(), 2);
        assert_eq!(retried, persisted);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn clear_retry_does_not_overwrite_newer_failure_publish() {
        let state = Mutex::new(ClipboardCapacityStatus::default());
        update_capacity_status_with(&state, true, 1, |_| Ok(())).expect("initial failure");
        let recovered =
            update_capacity_status_with(&state, false, 0, |_| Ok(())).expect("recovered status");
        assert_eq!(recovered.revision, 2);
        let recovered_snapshot = current_capacity_status(&state).expect("recovered snapshot");
        let failed = update_capacity_status_with(&state, true, 3, |_| Ok(()))
            .expect("newer failure publish");
        let calls = std::cell::Cell::new(0);

        let current =
            republish_capacity_status_if_unchanged_with(&state, &recovered_snapshot, |_| {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .expect("stale clear retry");

        assert_eq!(calls.get(), 0);
        assert_eq!(current, failed);
        assert!(current.blocked);
        assert_eq!(current.required_additional, 3);
        assert_eq!(current.revision, 3);
        assert_eq!(
            current_capacity_status(&state).expect("final status"),
            failed
        );
    }

    #[test]
    fn delayed_failure_publish_prevents_stale_recovery_clear() {
        let root = temp_dir("status-delayed-failure");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let state = Mutex::new(ClipboardCapacityStatus::default());
        update_capacity_status_with(&state, true, 1, |_| Ok(())).expect("initial failure");

        let recovered = clear_capacity_status_if_recovered_using(
            &state,
            &store,
            |_| {
                update_capacity_status_with(&state, true, 2, |_| Ok(()))
                    .expect("delayed newer failure publish");
                Ok(0)
            },
            |_| panic!("stale recovery must not emit clear"),
        )
        .expect("stale recovery check");

        assert!(recovered.blocked);
        assert_eq!(recovered.required_additional, 2);
        assert_eq!(recovered.revision, 2);
        assert_eq!(
            current_capacity_status(&state).expect("final status"),
            recovered
        );
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn recovery_requires_usage_plus_failed_additional_to_fit() {
        let root = temp_dir("status-required");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let filler = store.blob_dir().join("filler.tmp");
        let file = fs::File::create(&filler).expect("filler");
        file.set_len(MANAGED_BLOB_QUOTA).expect("sparse filler");
        let state = Mutex::new(ClipboardCapacityStatus::default());
        update_capacity_status_with(&state, true, 1, |_| Ok(())).expect("blocked status");

        let still_blocked = clear_capacity_status_if_recovered_with(&state, &store, |_| {
            panic!("exact quota plus required bytes must not clear")
        })
        .expect("recovery check");
        assert!(still_blocked.blocked);
        assert_eq!(still_blocked.required_additional, 1);

        let text_delete_did_not_help =
            clear_capacity_status_if_recovered_with(&state, &store, |_| {
                panic!("text-only delete must not clear blob capacity")
            })
            .expect("text delete recovery check");
        assert!(text_delete_did_not_help.blocked);

        file.set_len(MANAGED_BLOB_QUOTA - 1)
            .expect("release enough bytes");
        let cleared = clear_capacity_status_if_recovered_with(&state, &store, |_| Ok(()))
            .expect("clear after release");
        assert!(!cleared.blocked);
        assert_eq!(cleared.required_additional, 0);
        assert_eq!(cleared.revision, 2);
        drop(file);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn duplicate_image_success_does_not_clear_failed_capacity_status() {
        let root = temp_dir("status-duplicate");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let state = Mutex::new(ClipboardCapacityStatus::default());
        update_capacity_status_with(&state, true, 1, |_| Ok(())).expect("blocked status");

        let unchanged =
            clear_capacity_status_after_image_capture_with(&state, &store, false, |_| {
                panic!("duplicate touch must not clear or emit")
            })
            .expect("duplicate status");

        assert!(unchanged.blocked);
        assert_eq!(unchanged.revision, 1);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn recovery_update_holds_store_lease_before_later_failure_publish() {
        use std::sync::{mpsc, Arc};
        use std::thread;
        use std::time::Duration;

        let root = temp_dir("status-race");
        let store =
            Arc::new(ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store"));
        let state = Arc::new(Mutex::new(ClipboardCapacityStatus::default()));
        update_capacity_status_with(&state, true, 1, |_| Ok(())).expect("blocked status");
        let (clear_updated_tx, clear_updated_rx) = mpsc::channel();
        let (release_clear_tx, release_clear_rx) = mpsc::channel();
        let clear_store = Arc::clone(&store);
        let clear_state = Arc::clone(&state);
        let clear_thread = thread::spawn(move || {
            clear_capacity_status_if_recovered_with(&clear_state, &clear_store, |status| {
                assert!(!status.blocked);
                clear_updated_tx.send(()).expect("clear updated");
                release_clear_rx.recv().expect("release clear");
                Ok(())
            })
        });
        clear_updated_rx.recv().expect("clear reached update");

        let (writer_entered_tx, writer_entered_rx) = mpsc::channel();
        let writer_store = Arc::clone(&store);
        let writer = thread::spawn(move || {
            writer_store.with_write(|_, _| {
                writer_entered_tx.send(()).expect("writer entered");
                Ok(())
            })
        });
        assert!(writer_entered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_clear_tx.send(()).expect("release clear");
        clear_thread.join().expect("clear thread").expect("clear");
        writer_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer entered after clear");
        writer.join().expect("writer thread").expect("writer");

        let failed = update_capacity_status_with(&state, true, 2, |_| Ok(()))
            .expect("later failure publish");
        assert!(failed.blocked);
        assert_eq!(failed.required_additional, 2);
        assert_eq!(failed.revision, 3);
        assert_eq!(
            current_capacity_status(&state).expect("final query"),
            failed
        );
        drop(state);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn shared_path_is_not_projected_reclaimable_while_favorite_ref_remains() {
        let root = temp_dir("shared-favorite");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let shared_path = store.blob_dir().join("shared.bmp");
        fs::write(&shared_path, b"shared").expect("shared blob");
        let eligible = insert_image(&repository, &shared_path, "shared-eligible");
        let favorite = insert_image(&repository, &shared_path, "shared-favorite");
        repository
            .lock()
            .expect("repository lock")
            .toggle_favorite(&favorite.id)
            .expect("favorite");

        prune_for_capacity_with_quota(&repository, &store, 10_000, 0, 0, 0)
            .expect_err("favorite shared ref prevents reclaim");

        let repository_guard = repository.lock().expect("repository lock");
        for id in [&eligible.id, &favorite.id] {
            assert!(repository_guard
                .get_item(id)
                .expect("shared query")
                .is_some());
        }
        assert!(repository_guard
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository_guard);
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
