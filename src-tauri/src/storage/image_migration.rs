use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, ensure, Context};
use chrono::Utc;

use crate::blobs::store::{install_staged_locked, ImageBlobStore};
use crate::storage::repository::{ClipboardRepository, ImageMigrationMerge};

const IMAGE_MIGRATION_NAME: &str = "legacy-image-content-dedup-v1";
const IMAGE_QUOTA_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const BACKUP_COMPLETE_MARKER: &str = ".complete";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationOutcome {
    pub quota_blocked: bool,
    pub usage: u64,
}

trait MigrationIo {
    fn available_space(&self, path: &Path) -> anyhow::Result<u64>;
    fn copy_file(&self, from: &Path, to: &Path) -> anyhow::Result<u64>;
    fn quota_bytes(&self) -> u64;
}

struct SystemMigrationIo;

impl MigrationIo for SystemMigrationIo {
    fn available_space(&self, path: &Path) -> anyhow::Result<u64> {
        crate::blobs::store::available_space(path)
    }

    fn copy_file(&self, from: &Path, to: &Path) -> anyhow::Result<u64> {
        fs::copy(from, to).map_err(Into::into)
    }

    fn quota_bytes(&self) -> u64 {
        IMAGE_QUOTA_BYTES
    }
}

pub fn run_image_migration(
    repository: &Mutex<ClipboardRepository>,
    store: &ImageBlobStore,
) -> anyhow::Result<MigrationOutcome> {
    run_image_migration_with(repository, store, &SystemMigrationIo)
}

fn run_image_migration_with(
    repository: &Mutex<ClipboardRepository>,
    store: &ImageBlobStore,
    io: &impl MigrationIo,
) -> anyhow::Result<MigrationOutcome> {
    store.with_write(|blob_dir, stage_root| {
        let record = migration_record(repository)?;
        if record
            .as_ref()
            .is_some_and(|record| record.state == "complete")
        {
            return migration_outcome(blob_dir, io.quota_bytes());
        }

        let database_path = repository_lock(repository)?.database_path().to_path_buf();
        let record = match record {
            Some(record) => record,
            None => {
                let backup_path = backup_path_for(&database_path)?;
                repository_lock(repository)?
                    .reserve_migration_backup(IMAGE_MIGRATION_NAME, &backup_path)?
            }
        };
        let backup_path = record
            .backup_path
            .ok_or_else(|| anyhow!("pending image migration has no backup path"))?;
        match record.state.as_str() {
            "pending" => ensure_backup(repository, &database_path, &backup_path, io)?,
            "cleanup_pending" => {}
            state => return Err(anyhow!("invalid image migration state: {state}")),
        }

        if record.state == "pending" {
            let rows = repository_lock(repository)?.active_image_rows()?;
            let scan = scan_images(blob_dir, rows)?;
            let available = io.available_space(stage_root)?;
            ensure!(
                available >= scan.temporary_bytes,
                "insufficient free space for image migration: required {}, available {}",
                scan.temporary_bytes,
                available
            );
            let migration_stage = prepare_migration_stage(stage_root)?;
            let merges = prepare_canonical_images(blob_dir, &migration_stage, scan.groups)?;
            repository_lock(repository)?.commit_image_migration(IMAGE_MIGRATION_NAME, &merges)?;
        }
        cleanup_pending(repository, blob_dir)?;
        remove_migration_stage(stage_root)?;
        repository_lock(repository)?.set_migration_state(IMAGE_MIGRATION_NAME, "complete")?;
        migration_outcome(blob_dir, io.quota_bytes())
    })
}

struct ScannedImage {
    row: crate::storage::repository::MigrationImageRow,
    source_path: PathBuf,
    source_len: u64,
    decoded_bytes: u64,
}

struct MigrationScan {
    groups: BTreeMap<String, Vec<ScannedImage>>,
    temporary_bytes: u64,
}

fn scan_images(
    blob_dir: &Path,
    rows: Vec<crate::storage::repository::MigrationImageRow>,
) -> anyhow::Result<MigrationScan> {
    let mut groups = BTreeMap::<String, Vec<ScannedImage>>::new();
    for row in rows {
        let stored_path = row
            .content_path
            .as_deref()
            .ok_or_else(|| anyhow!("active image {} has no content path", row.id))?;
        let joined_path = if stored_path.is_absolute() {
            stored_path.to_path_buf()
        } else {
            blob_dir.join(stored_path)
        };
        let metadata = fs::symlink_metadata(&joined_path)
            .with_context(|| format!("read legacy image metadata {}", joined_path.display()))?;
        ensure!(
            metadata.file_type().is_file(),
            "legacy image is not a regular file: {}",
            joined_path.display()
        );
        let source_path = joined_path
            .canonicalize()
            .with_context(|| format!("resolve legacy image {}", joined_path.display()))?;
        ensure!(
            source_path.parent() == Some(blob_dir),
            "legacy image path is outside blob directory: {}",
            source_path.display()
        );
        let dib = crate::blobs::read_dib_from_bmp_file(&source_path)
            .with_context(|| format!("read legacy image {}", source_path.display()))?;
        let identity = crate::blobs::image::image_identity_from_dib(&dib)
            .with_context(|| format!("decode legacy image {}", source_path.display()))?;
        if let Some(expected_hash) = canonical_hash_from_filename(&source_path) {
            ensure!(
                identity.content_hash == expected_hash,
                "existing canonical semantic hash mismatch for {}: expected {}, got {}",
                source_path.display(),
                expected_hash,
                identity.content_hash
            );
        }
        let pixel_count = u64::from(identity.width)
            .checked_mul(u64::from(identity.height))
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| anyhow!("decoded image byte count overflow"))?;
        groups
            .entry(identity.content_hash.clone())
            .or_default()
            .push(ScannedImage {
                row,
                source_path,
                source_len: metadata.len(),
                decoded_bytes: pixel_count,
            });
    }

    let mut temporary_bytes = 0u64;
    for (content_hash, images) in &groups {
        let canonical_bmp = crate::blobs::image::canonical_bmp_path(blob_dir, content_hash)?;
        let canonical_thumbnail =
            crate::blobs::image::canonical_thumbnail_path(blob_dir, content_hash)?;
        let first = images
            .first()
            .ok_or_else(|| anyhow!("image migration group is empty"))?;
        if canonical_bmp.exists() {
            validate_canonical_bmp(&canonical_bmp, content_hash)?;
        }
        if canonical_thumbnail.exists() {
            let expected =
                crate::blobs::image::decoded_thumbnail_for_hash(&first.source_path, content_hash)?;
            crate::blobs::image::validate_thumbnail(&canonical_thumbnail, &expected)?;
        }
        if !canonical_bmp.exists() || !canonical_thumbnail.exists() {
            temporary_bytes = temporary_bytes
                .checked_add(first.source_len)
                .and_then(|value| value.checked_add(first.decoded_bytes))
                .ok_or_else(|| anyhow!("temporary image byte count overflow"))?;
        }
    }
    Ok(MigrationScan {
        groups,
        temporary_bytes,
    })
}

fn canonical_hash_from_filename(path: &Path) -> Option<&str> {
    if path.extension().and_then(|value| value.to_str()) != Some("bmp") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    (stem.len() == 64
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(stem)
}

fn prepare_migration_stage(stage_root: &Path) -> anyhow::Result<PathBuf> {
    let migration_stage = stage_root.join("legacy-image-dedup-v1");
    if migration_stage.exists() {
        let metadata = fs::symlink_metadata(&migration_stage)?;
        ensure!(
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
            "migration stage is not a regular directory"
        );
        let resolved = migration_stage.canonicalize()?;
        ensure!(
            resolved.parent() == Some(stage_root),
            "migration stage is outside managed stage root"
        );
        fs::remove_dir_all(&resolved)
            .with_context(|| format!("remove stale migration stage {}", resolved.display()))?;
    }
    fs::create_dir(&migration_stage)
        .with_context(|| format!("create migration stage {}", migration_stage.display()))?;
    migration_stage.canonicalize().map_err(Into::into)
}

fn prepare_canonical_images(
    blob_dir: &Path,
    migration_stage: &Path,
    groups: BTreeMap<String, Vec<ScannedImage>>,
) -> anyhow::Result<Vec<ImageMigrationMerge>> {
    let mut merges = Vec::with_capacity(groups.len());
    for (content_hash, mut images) in groups {
        images.sort_by(|left, right| {
            left.row
                .created_at
                .cmp(&right.row.created_at)
                .then_with(|| left.row.id.cmp(&right.row.id))
        });
        let first = images
            .first()
            .ok_or_else(|| anyhow!("image migration group is empty"))?;
        let canonical_bmp = crate::blobs::image::canonical_bmp_path(blob_dir, &content_hash)?;
        let canonical_thumbnail =
            crate::blobs::image::canonical_thumbnail_path(blob_dir, &content_hash)?;
        if !canonical_bmp.exists() || !canonical_thumbnail.exists() {
            let dib = crate::blobs::read_dib_from_bmp_file(&first.source_path)?;
            let staged = crate::blobs::image::stage_dib(migration_stage, dib)?;
            ensure!(
                staged.content_hash() == content_hash,
                "legacy image changed after semantic scan"
            );
            install_staged_locked(blob_dir, migration_stage, staged)?;
        }
        validate_canonical_bmp(&canonical_bmp, &content_hash)?;
        let expected_thumbnail =
            crate::blobs::image::decoded_thumbnail_for_hash(&canonical_bmp, &content_hash)?;
        crate::blobs::image::validate_thumbnail(&canonical_thumbnail, &expected_thumbnail)?;

        let retained = &images[0].row;
        let duplicate_ids = images
            .iter()
            .skip(1)
            .map(|image| image.row.id.clone())
            .collect();
        let mut obsolete_paths = BTreeSet::new();
        for image in &images {
            if image.source_path != canonical_bmp {
                obsolete_paths.insert(image.source_path.clone());
            }
        }
        merges.push(ImageMigrationMerge {
            retained_id: retained.id.clone(),
            duplicate_ids,
            content_hash,
            content_path: canonical_bmp.clone(),
            favorite: images.iter().any(|image| image.row.favorite),
            pinned: images.iter().any(|image| image.row.pinned),
            created_at: images
                .iter()
                .map(|image| image.row.created_at)
                .min()
                .ok_or_else(|| anyhow!("image group has no created timestamp"))?,
            updated_at: images
                .iter()
                .map(|image| image.row.updated_at)
                .max()
                .ok_or_else(|| anyhow!("image group has no updated timestamp"))?,
            sort_rank: images
                .iter()
                .map(|image| image.row.sort_rank)
                .max()
                .ok_or_else(|| anyhow!("image group has no sort rank"))?,
            size_bytes: i64::try_from(fs::metadata(&canonical_bmp)?.len()).unwrap_or(i64::MAX),
            obsolete_paths: obsolete_paths.into_iter().collect(),
        });
    }
    Ok(merges)
}

fn cleanup_pending(repository: &Mutex<ClipboardRepository>, blob_dir: &Path) -> anyhow::Result<()> {
    let (pending_paths, active_paths) = {
        let repository = repository_lock(repository)?;
        (
            repository.pending_cleanup_paths()?,
            repository.active_blob_paths()?,
        )
    };
    let active_paths = active_paths
        .into_iter()
        .flat_map(|path| {
            let thumbnail = crate::blobs::thumbnail_path_for(&path);
            [path, thumbnail]
        })
        .collect::<HashSet<_>>();
    let mut first_error = None;
    for path in pending_paths {
        let result = (|| {
            ensure!(
                path.is_absolute() && path.parent() == Some(blob_dir),
                "cleanup path outside managed blob directory: {}",
                path.display()
            );
            ensure!(
                !active_paths.contains(&path),
                "cleanup path is still active: {}",
                path.display()
            );
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    ensure!(
                        metadata.file_type().is_file() || metadata.file_type().is_symlink(),
                        "cleanup path is not a file: {}",
                        path.display()
                    );
                    fs::remove_file(&path)
                        .with_context(|| format!("remove obsolete image {}", path.display()))?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            repository_lock(repository)?.complete_cleanup_path(&path)?;
            Ok::<(), anyhow::Error>(())
        })();
        if let Err(error) = result {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    ensure!(
        repository_lock(repository)?
            .pending_cleanup_paths()?
            .is_empty(),
        "image migration cleanup queue is not empty"
    );
    Ok(())
}

fn remove_migration_stage(stage_root: &Path) -> anyhow::Result<()> {
    let migration_stage = stage_root.join("legacy-image-dedup-v1");
    match fs::symlink_metadata(&migration_stage) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                "migration stage is not a regular directory"
            );
            let resolved = migration_stage.canonicalize()?;
            ensure!(
                resolved.parent() == Some(stage_root),
                "migration stage is outside managed stage root"
            );
            fs::remove_dir_all(resolved)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn validate_canonical_bmp(path: &Path, expected_hash: &str) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("read canonical image metadata {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "canonical image is not a regular file: {}",
        path.display()
    );
    crate::blobs::image::decoded_thumbnail_for_hash(path, expected_hash).map(|_| ())
}

fn repository_lock(
    repository: &Mutex<ClipboardRepository>,
) -> anyhow::Result<std::sync::MutexGuard<'_, ClipboardRepository>> {
    repository
        .lock()
        .map_err(|error| anyhow!("repository lock poisoned: {error}"))
}

fn migration_record(
    repository: &Mutex<ClipboardRepository>,
) -> anyhow::Result<Option<crate::storage::repository::MigrationRecord>> {
    repository_lock(repository)?.migration_record(IMAGE_MIGRATION_NAME)
}

fn backup_path_for(database_path: &Path) -> anyhow::Result<PathBuf> {
    let parent = database_path
        .parent()
        .ok_or_else(|| anyhow!("database path has no parent"))?;
    let file_name = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("database path has no valid filename"))?;
    Ok(parent.join(format!(
        "{file_name}.image-migration-backup-{}",
        Utc::now().timestamp_micros()
    )))
}

fn ensure_backup(
    repository: &Mutex<ClipboardRepository>,
    database_path: &Path,
    backup_path: &Path,
    io: &impl MigrationIo,
) -> anyhow::Result<()> {
    ensure!(
        backup_path.is_absolute() && backup_path.parent() == database_path.parent(),
        "migration backup path is not a database sibling"
    );
    fs::create_dir_all(backup_path)
        .with_context(|| format!("create migration backup {}", backup_path.display()))?;
    if backup_path.join(BACKUP_COMPLETE_MARKER).is_file() {
        return Ok(());
    }

    repository_lock(repository)?.checkpoint_wal()?;
    let mut sources = vec![database_path.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let sidecar = sidecar_path(database_path, suffix);
        if sidecar.exists() {
            sources.push(sidecar);
        }
    }
    for source in sources {
        let destination = backup_path.join(
            source
                .file_name()
                .ok_or_else(|| anyhow!("backup source has no filename"))?,
        );
        io.copy_file(&source, &destination).with_context(|| {
            format!(
                "copy migration backup file {} to {}",
                source.display(),
                destination.display()
            )
        })?;
        OpenOptions::new()
            .write(true)
            .open(&destination)
            .with_context(|| format!("open copied backup {}", destination.display()))?
            .sync_all()
            .with_context(|| format!("flush copied backup {}", destination.display()))?;
    }
    let marker_path = backup_path.join(BACKUP_COMPLETE_MARKER);
    let mut marker = File::create(&marker_path)
        .with_context(|| format!("create backup marker {}", marker_path.display()))?;
    marker.write_all(b"complete\n")?;
    marker.sync_all()?;
    Ok(())
}

fn sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(database_path.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

fn migration_outcome(blob_dir: &Path, quota_bytes: u64) -> anyhow::Result<MigrationOutcome> {
    let usage = crate::blobs::store::managed_usage(blob_dir)?;
    Ok(MigrationOutcome {
        quota_blocked: usage > quota_bytes,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use anyhow::anyhow;
    use rusqlite::Connection;
    use uuid::Uuid;

    use crate::blobs::image::{image_identity_from_dib, stage_dib};
    use crate::blobs::store::ImageBlobStore;
    use crate::storage::repository::ClipboardRepository;

    use super::{run_image_migration_with, MigrationIo, IMAGE_MIGRATION_NAME, IMAGE_QUOTA_BYTES};

    struct TestIo {
        free_space: u64,
        quota_bytes: u64,
        fail_next_copy: Cell<bool>,
        forbid_copy: bool,
    }

    impl TestIo {
        fn real() -> Self {
            Self {
                free_space: u64::MAX,
                quota_bytes: IMAGE_QUOTA_BYTES,
                fail_next_copy: Cell::new(false),
                forbid_copy: false,
            }
        }
    }

    impl MigrationIo for TestIo {
        fn available_space(&self, _path: &Path) -> anyhow::Result<u64> {
            Ok(self.free_space)
        }

        fn copy_file(&self, from: &Path, to: &Path) -> anyhow::Result<u64> {
            if self.forbid_copy {
                return Err(anyhow!("copy must not run"));
            }
            if self.fail_next_copy.replace(false) {
                return Err(anyhow!("injected backup copy failure"));
            }
            Ok(fs::copy(from, to)?)
        }

        fn quota_bytes(&self) -> u64 {
            self.quota_bytes
        }
    }

    struct Fixture {
        root: PathBuf,
        database_path: PathBuf,
        repository: Mutex<ClipboardRepository>,
        store: ImageBlobStore,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "super-clipboard-image-migration-{label}-{}",
                Uuid::new_v4()
            ));
            fs::create_dir_all(&root).expect("fixture root");
            let database_path = root.join("history.sqlite3");
            let repository =
                Mutex::new(ClipboardRepository::open(database_path.clone()).expect("repository"));
            let store = ImageBlobStore::new(root.join("blobs"), root.join("blob-stage"))
                .expect("image store");
            Self {
                root,
                database_path,
                repository,
                store,
            }
        }

        fn migration_state(&self) -> (String, PathBuf) {
            let connection = Connection::open(&self.database_path).expect("state connection");
            connection
                .query_row(
                    "SELECT state, backup_path FROM schema_migrations WHERE name = ?1",
                    [IMAGE_MIGRATION_NAME],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            PathBuf::from(row.get::<_, String>(1)?),
                        ))
                    },
                )
                .expect("migration state")
        }

        fn dib32(header_size: usize, pixel: [u8; 4]) -> Vec<u8> {
            let mut dib = vec![0u8; header_size];
            dib[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
            dib[4..8].copy_from_slice(&1i32.to_le_bytes());
            dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
            dib[12..14].copy_from_slice(&1u16.to_le_bytes());
            dib[14..16].copy_from_slice(&32u16.to_le_bytes());
            dib[20..24].copy_from_slice(&4u32.to_le_bytes());
            dib.extend_from_slice(&pixel);
            dib
        }

        fn write_legacy_bmp(&self, filename: &str, dib: Vec<u8>) -> PathBuf {
            let staged = stage_dib(self.store.stage_dir(), dib).expect("stage legacy BMP");
            let path = self.store.blob_dir().join(filename);
            fs::copy(staged.bmp_path(), &path).expect("copy legacy BMP");
            fs::remove_dir_all(staged.stage_dir()).expect("remove source stage");
            path
        }

        fn insert_image_row(
            &self,
            id: &str,
            path: &Path,
            created_at: i64,
            updated_at: i64,
            sort_rank: i64,
            favorite: bool,
            pinned: bool,
        ) {
            let connection = Connection::open(&self.database_path).expect("write connection");
            connection
                .execute(
                    "INSERT INTO clipboard_items
                     (id, hash, item_type, content_path, content_hash, preview, favorite, pinned,
                      size_bytes, sort_rank, created_at, updated_at)
                     VALUES (?1, ?2, 'image', ?3, NULL, ?4, ?5, ?6, 1, ?7, ?8, ?9)",
                    rusqlite::params![
                        id,
                        format!("legacy:{id}"),
                        path.to_string_lossy(),
                        format!("preview {id}"),
                        i64::from(favorite),
                        i64::from(pinned),
                        sort_rank,
                        created_at,
                        updated_at
                    ],
                )
                .expect("insert image row");
            connection
                .execute(
                    "INSERT INTO clipboard_items_fts(id, preview, content) VALUES (?1, ?2, NULL)",
                    rusqlite::params![id, format!("preview {id}")],
                )
                .expect("insert image FTS");
        }

        fn set_content_hash(&self, id: &str, content_hash: &str) {
            Connection::open(&self.database_path)
                .expect("identity connection")
                .execute(
                    "UPDATE clipboard_items SET hash = ?1, content_hash = ?2 WHERE id = ?3",
                    rusqlite::params![format!("image:{content_hash}"), content_hash, id],
                )
                .expect("set content hash");
        }

        fn swap_stored_identities(
            &self,
            first_id: &str,
            first_actual_hash: &str,
            second_id: &str,
            second_actual_hash: &str,
        ) {
            let mut connection =
                Connection::open(&self.database_path).expect("identity connection");
            let transaction = connection.transaction().expect("identity transaction");
            transaction
                .execute(
                    "UPDATE clipboard_items SET hash = ?1, content_hash = NULL WHERE id = ?2",
                    rusqlite::params![format!("migration-temp:{first_id}"), first_id],
                )
                .expect("temporarily clear first identity");
            transaction
                .execute(
                    "UPDATE clipboard_items SET hash = ?1, content_hash = ?2 WHERE id = ?3",
                    rusqlite::params![
                        format!("image:{first_actual_hash}"),
                        first_actual_hash,
                        second_id
                    ],
                )
                .expect("swap second identity");
            transaction
                .execute(
                    "UPDATE clipboard_items SET hash = ?1, content_hash = ?2 WHERE id = ?3",
                    rusqlite::params![
                        format!("image:{second_actual_hash}"),
                        second_actual_hash,
                        first_id
                    ],
                )
                .expect("swap first identity");
            transaction.commit().expect("commit swapped identities");
        }

        fn row_reference(&self, id: &str) -> (String, Option<String>, String) {
            Connection::open(&self.database_path)
                .expect("read connection")
                .query_row(
                    "SELECT hash, content_hash, content_path FROM clipboard_items WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("row reference")
        }

        fn merged_row(
            &self,
            id: &str,
        ) -> (
            bool,
            bool,
            i64,
            i64,
            i64,
            String,
            Option<String>,
            String,
            Option<i64>,
        ) {
            Connection::open(&self.database_path)
                .expect("read connection")
                .query_row(
                    "SELECT favorite, pinned, created_at, updated_at, sort_rank, hash,
                            content_hash, content_path, deleted_at
                     FROM clipboard_items WHERE id = ?1",
                    [id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)? == 1,
                            row.get::<_, i64>(1)? == 1,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                        ))
                    },
                )
                .expect("merged row")
        }

        fn fts_count(&self, id: &str) -> i64 {
            Connection::open(&self.database_path)
                .expect("FTS connection")
                .query_row(
                    "SELECT COUNT(*) FROM clipboard_items_fts WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .expect("FTS count")
        }

        fn cleanup_paths(&self) -> Vec<PathBuf> {
            let connection = Connection::open(&self.database_path).expect("cleanup connection");
            let mut statement = connection
                .prepare("SELECT path FROM blob_cleanup_queue ORDER BY path")
                .expect("cleanup statement");
            statement
                .query_map([], |row| row.get::<_, String>(0).map(PathBuf::from))
                .expect("cleanup rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("cleanup paths")
        }

        fn install_merge_failure_trigger(&self) {
            Connection::open(&self.database_path)
                .expect("trigger connection")
                .execute_batch(
                    "CREATE TRIGGER fail_image_migration_merge
                     BEFORE UPDATE OF content_hash ON clipboard_items
                     BEGIN
                       SELECT RAISE(ABORT, 'injected migration transaction failure');
                     END;",
                )
                .expect("install merge trigger");
        }

        fn remove_merge_failure_trigger(&self) {
            Connection::open(&self.database_path)
                .expect("trigger connection")
                .execute_batch("DROP TRIGGER fail_image_migration_merge")
                .expect("remove merge trigger");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn backs_up_database_and_existing_wal_and_shm_before_completing() {
        let fixture = Fixture::new("backup");
        let wal_path = PathBuf::from(format!("{}-wal", fixture.database_path.display()));
        let shm_path = PathBuf::from(format!("{}-shm", fixture.database_path.display()));
        assert!(wal_path.is_file(), "test requires an existing WAL");
        assert!(shm_path.is_file(), "test requires an existing SHM");

        run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect("migration");

        let (state, backup_path) = fixture.migration_state();
        assert_eq!(state, "complete");
        for source in [&fixture.database_path, &wal_path, &shm_path] {
            let backup_file = backup_path.join(source.file_name().expect("source filename"));
            assert!(backup_file.is_file(), "missing {}", backup_file.display());
        }
    }

    #[test]
    fn backup_failure_keeps_pending_and_retry_reuses_reserved_path() {
        let fixture = Fixture::new("backup-retry");
        let failing = TestIo {
            free_space: u64::MAX,
            quota_bytes: IMAGE_QUOTA_BYTES,
            fail_next_copy: Cell::new(true),
            forbid_copy: false,
        };

        let error = run_image_migration_with(&fixture.repository, &fixture.store, &failing)
            .expect_err("backup must fail");
        assert!(format!("{error:#}").contains("injected backup copy failure"));
        let (state, first_backup_path) = fixture.migration_state();
        assert_eq!(state, "pending");

        run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect("retry migration");
        let (state, retried_backup_path) = fixture.migration_state();
        assert_eq!(state, "complete");
        assert_eq!(retried_backup_path, first_backup_path);
    }

    #[test]
    fn completed_migration_does_not_repeat_backup() {
        let fixture = Fixture::new("complete");
        run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect("first migration");
        let (_, first_backup_path) = fixture.migration_state();
        let forbid_copy = TestIo {
            free_space: u64::MAX,
            quota_bytes: IMAGE_QUOTA_BYTES,
            fail_next_copy: Cell::new(false),
            forbid_copy: true,
        };

        run_image_migration_with(&fixture.repository, &fixture.store, &forbid_copy)
            .expect("completed migration");

        assert_eq!(fixture.migration_state().1, first_backup_path);
    }

    #[test]
    fn missing_legacy_blob_stops_with_history_unchanged() {
        let fixture = Fixture::new("missing");
        let missing = fixture.store.blob_dir().join("missing.bmp");
        fixture.insert_image_row("legacy", &missing, 1, 2, 2, false, false);
        let before = fixture.row_reference("legacy");

        let error = run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect_err("missing blob must stop migration");

        assert!(format!("{error:#}").contains("missing.bmp"));
        assert_eq!(fixture.row_reference("legacy"), before);
        assert_eq!(fixture.migration_state().0, "pending");
    }

    #[test]
    fn free_space_preflight_fails_before_stage_or_canonical_creation() {
        let fixture = Fixture::new("preflight");
        let dib = Fixture::dib32(40, [30, 20, 10, 255]);
        let content_hash = image_identity_from_dib(&dib)
            .expect("image identity")
            .content_hash;
        let legacy = fixture.write_legacy_bmp("legacy.bmp", dib);
        fixture.insert_image_row("legacy", &legacy, 1, 2, 2, false, false);
        let before = fixture.row_reference("legacy");
        let no_space = TestIo {
            free_space: 0,
            quota_bytes: IMAGE_QUOTA_BYTES,
            fail_next_copy: Cell::new(false),
            forbid_copy: false,
        };

        let error = run_image_migration_with(&fixture.repository, &fixture.store, &no_space)
            .expect_err("preflight must reject insufficient space");

        assert!(format!("{error:#}").contains("insufficient"));
        assert_eq!(fixture.row_reference("legacy"), before);
        assert!(!fixture
            .store
            .blob_dir()
            .join(format!("{content_hash}.bmp"))
            .exists());
        assert!(!fixture
            .store
            .stage_dir()
            .join("legacy-image-dedup-v1")
            .exists());
        assert_eq!(fixture.migration_state().0, "pending");
    }

    #[test]
    fn corrupted_existing_canonical_stops_with_references_unchanged() {
        let fixture = Fixture::new("corrupt-canonical");
        let expected_hash = image_identity_from_dib(&Fixture::dib32(40, [1, 2, 3, 255]))
            .expect("expected identity")
            .content_hash;
        let corrupt_path = fixture.write_legacy_bmp(
            &format!("{expected_hash}.bmp"),
            Fixture::dib32(40, [9, 8, 7, 255]),
        );
        fixture.insert_image_row("canonical", &corrupt_path, 1, 2, 2, false, false);
        fixture.set_content_hash("canonical", &expected_hash);
        let before = fixture.row_reference("canonical");

        let error = run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect_err("corrupt canonical must stop migration");

        assert!(format!("{error:#}").contains("semantic hash mismatch"));
        assert_eq!(fixture.row_reference("canonical"), before);
        assert_eq!(fixture.migration_state().0, "pending");
    }

    #[test]
    fn transaction_neutralizes_swapped_stored_identities_before_final_updates() {
        let fixture = Fixture::new("swapped-identities");
        let first_dib = Fixture::dib32(40, [1, 2, 3, 255]);
        let second_dib = Fixture::dib32(40, [9, 8, 7, 255]);
        let first_hash = image_identity_from_dib(&first_dib)
            .expect("first identity")
            .content_hash;
        let second_hash = image_identity_from_dib(&second_dib)
            .expect("second identity")
            .content_hash;
        let first_path = fixture.write_legacy_bmp(&format!("{first_hash}.bmp"), first_dib);
        let second_path = fixture.write_legacy_bmp(&format!("{second_hash}.bmp"), second_dib);
        fixture.insert_image_row("first", &first_path, 1, 2, 2, false, false);
        fixture.insert_image_row("second", &second_path, 2, 3, 3, false, false);
        fixture.swap_stored_identities("first", &first_hash, "second", &second_hash);

        run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect("migration repairs swapped identities");

        assert_eq!(
            fixture.row_reference("first").0,
            format!("image:{first_hash}")
        );
        assert_eq!(fixture.row_reference("first").1, Some(first_hash));
        assert_eq!(
            fixture.row_reference("second").0,
            format!("image:{second_hash}")
        );
        assert_eq!(fixture.row_reference("second").1, Some(second_hash));
    }

    #[test]
    fn merge_transaction_preserves_metadata_and_durable_cleanup_work() {
        let fixture = Fixture::new("merge");
        let first_dib = Fixture::dib32(40, [30, 20, 10, 255]);
        let second_dib = Fixture::dib32(124, [30, 20, 10, 255]);
        let content_hash = image_identity_from_dib(&first_dib)
            .expect("first identity")
            .content_hash;
        assert_eq!(
            image_identity_from_dib(&second_dib)
                .expect("second identity")
                .content_hash,
            content_hash
        );
        let first_path = fixture.write_legacy_bmp("old-a.bmp", first_dib);
        let second_path = fixture.write_legacy_bmp("old-b.bmp", second_dib);
        fixture.insert_image_row("earliest", &first_path, 10, 300, 20, false, true);
        fixture.insert_image_row("duplicate", &second_path, 20, 200, 400, true, false);
        let failing_thumbnail = fixture.store.blob_dir().join("old-a.thumb.png");
        fs::create_dir(&failing_thumbnail).expect("blocking thumbnail directory");

        let error = run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect_err("cleanup must fail after commit");

        assert!(format!("{error:#}").contains("old-a.thumb.png"));
        let canonical = fixture.store.blob_dir().join(format!("{content_hash}.bmp"));
        assert!(canonical.is_file(), "canonical BMP must precede commit");
        assert!(fixture
            .store
            .blob_dir()
            .join(format!("{content_hash}.thumb.png"))
            .is_file());
        assert_eq!(
            fixture.merged_row("earliest"),
            (
                true,
                true,
                10,
                300,
                400,
                format!("image:{content_hash}"),
                Some(content_hash),
                canonical.to_string_lossy().into_owned(),
                None,
            )
        );
        assert!(fixture.merged_row("duplicate").8.is_some());
        assert_eq!(fixture.fts_count("duplicate"), 0);
        assert_eq!(fixture.migration_state().0, "cleanup_pending");
        assert_eq!(fixture.cleanup_paths(), vec![failing_thumbnail]);
    }

    #[test]
    fn pending_transaction_failure_reuses_canonical_and_backup_on_retry() {
        let fixture = Fixture::new("transaction-retry");
        let dib = Fixture::dib32(40, [3, 2, 1, 255]);
        let content_hash = image_identity_from_dib(&dib)
            .expect("identity")
            .content_hash;
        let legacy = fixture.write_legacy_bmp("legacy.bmp", dib);
        fixture.insert_image_row("legacy", &legacy, 1, 2, 2, false, false);
        let before = fixture.row_reference("legacy");
        fixture.install_merge_failure_trigger();

        let error = run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect_err("transaction must fail");

        assert!(format!("{error:#}").contains("injected migration transaction failure"));
        assert_eq!(fixture.row_reference("legacy"), before);
        assert!(fixture.cleanup_paths().is_empty());
        assert_eq!(fixture.migration_state().0, "pending");
        assert!(fixture
            .store
            .blob_dir()
            .join(format!("{content_hash}.bmp"))
            .is_file());

        fixture.remove_merge_failure_trigger();
        let forbid_copy = TestIo {
            free_space: u64::MAX,
            quota_bytes: IMAGE_QUOTA_BYTES,
            fail_next_copy: Cell::new(false),
            forbid_copy: true,
        };
        run_image_migration_with(&fixture.repository, &fixture.store, &forbid_copy)
            .expect("pending retry");
        assert_eq!(fixture.migration_state().0, "complete");
        assert_eq!(
            fixture.row_reference("legacy").0,
            format!("image:{content_hash}")
        );
    }

    #[test]
    fn cleanup_pending_resume_retries_without_backup_or_legacy_rescan() {
        let fixture = Fixture::new("cleanup-resume");
        let dib = Fixture::dib32(40, [9, 8, 7, 255]);
        let legacy = fixture.write_legacy_bmp("resume.bmp", dib);
        fixture.insert_image_row("legacy", &legacy, 1, 2, 2, false, false);
        let failing_thumbnail = fixture.store.blob_dir().join("resume.thumb.png");
        fs::create_dir(&failing_thumbnail).expect("blocking thumbnail directory");

        run_image_migration_with(&fixture.repository, &fixture.store, &TestIo::real())
            .expect_err("first cleanup must fail");
        assert_eq!(fixture.migration_state().0, "cleanup_pending");
        assert!(
            !legacy.exists(),
            "successful cleanup work must stay durable"
        );
        assert_eq!(fixture.cleanup_paths(), vec![failing_thumbnail.clone()]);

        fs::remove_dir(&failing_thumbnail).expect("unblock cleanup");
        let forbid_copy = TestIo {
            free_space: u64::MAX,
            quota_bytes: IMAGE_QUOTA_BYTES,
            fail_next_copy: Cell::new(false),
            forbid_copy: true,
        };
        run_image_migration_with(&fixture.repository, &fixture.store, &forbid_copy)
            .expect("cleanup resume");

        assert_eq!(fixture.migration_state().0, "complete");
        assert!(fixture.cleanup_paths().is_empty());
        assert!(!fixture
            .store
            .stage_dir()
            .join("legacy-image-dedup-v1")
            .exists());
    }

    #[test]
    fn post_cleanup_usage_sets_quota_blocked_startup_gate() {
        let fixture = Fixture::new("quota");
        fs::write(fixture.store.blob_dir().join("managed.bin"), b"12").expect("managed file");
        let tiny_quota = TestIo {
            free_space: u64::MAX,
            quota_bytes: 1,
            fail_next_copy: Cell::new(false),
            forbid_copy: false,
        };

        let outcome = run_image_migration_with(&fixture.repository, &fixture.store, &tiny_quota)
            .expect("migration outcome");

        assert_eq!(outcome.usage, 2);
        assert!(outcome.quota_blocked);
    }
}
