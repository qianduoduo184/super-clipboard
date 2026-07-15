use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{anyhow, ensure, Context};

use super::image::{
    canonical_bmp_path, canonical_thumbnail_path, decoded_thumbnail_for_hash, validate_thumbnail,
    StagedImage,
};

#[derive(Debug)]
pub struct ImageBlobStore {
    blob_dir: PathBuf,
    stage_dir: PathBuf,
    lease: RwLock<()>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledImage {
    content_hash: String,
    bmp_path: PathBuf,
    thumbnail_path: PathBuf,
    created_paths: Vec<PathBuf>,
}

impl InstalledImage {
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn bmp_path(&self) -> &Path {
        &self.bmp_path
    }

    pub fn thumbnail_path(&self) -> &Path {
        &self.thumbnail_path
    }

    pub fn created_paths(&self) -> &[PathBuf] {
        &self.created_paths
    }
}

impl ImageBlobStore {
    pub fn new(blob_dir: PathBuf, stage_dir: PathBuf) -> anyhow::Result<Self> {
        ensure!(
            blob_dir != stage_dir
                && blob_dir.parent().is_some()
                && blob_dir.parent() == stage_dir.parent(),
            "image blob and stage directories must be distinct siblings"
        );
        fs::create_dir_all(&blob_dir)
            .with_context(|| format!("create image blob directory {}", blob_dir.display()))?;
        fs::create_dir_all(&stage_dir)
            .with_context(|| format!("create image stage directory {}", stage_dir.display()))?;
        let blob_dir = blob_dir.canonicalize()?;
        let stage_dir = stage_dir.canonicalize()?;
        ensure!(
            blob_dir != stage_dir && blob_dir.parent() == stage_dir.parent(),
            "image blob and stage directories must resolve to distinct siblings"
        );
        Ok(Self {
            blob_dir,
            stage_dir,
            lease: RwLock::new(()),
        })
    }

    pub fn blob_dir(&self) -> &Path {
        &self.blob_dir
    }

    pub fn stage_dir(&self) -> &Path {
        &self.stage_dir
    }

    pub fn with_read<T>(&self, f: impl FnOnce(&Path) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _lease = self.read_lease()?;
        f(&self.blob_dir)
    }

    pub fn with_write<T>(
        &self,
        f: impl FnOnce(&Path, &Path) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let _lease = self.write_lease()?;
        f(&self.blob_dir, &self.stage_dir)
    }

    pub fn install_staged_with<T>(
        &self,
        staged: StagedImage,
        persist: impl FnOnce(&InstalledImage) -> anyhow::Result<T>,
        is_referenced: impl FnOnce(&InstalledImage) -> anyhow::Result<bool>,
    ) -> anyhow::Result<T> {
        let _lease = self.write_lease()?;
        let installed = install_staged_locked(&self.blob_dir, &self.stage_dir, staged)?;
        match persist(&installed) {
            Ok(value) => Ok(value),
            Err(persist_error) => Err(clean_after_persist_failure(
                &installed,
                persist_error,
                is_referenced,
            )),
        }
    }

    pub fn managed_usage(&self) -> anyhow::Result<u64> {
        self.with_read(managed_usage)
    }

    pub fn available_space(&self) -> anyhow::Result<u64> {
        self.with_read(available_space)
    }

    fn read_lease(&self) -> anyhow::Result<std::sync::RwLockReadGuard<'_, ()>> {
        self.lease
            .read()
            .map_err(|_| anyhow!("image blob read lease is poisoned"))
    }

    fn write_lease(&self) -> anyhow::Result<std::sync::RwLockWriteGuard<'_, ()>> {
        self.lease
            .write()
            .map_err(|_| anyhow!("image blob write lease is poisoned"))
    }
}

pub fn managed_usage(blob_dir: &Path) -> anyhow::Result<u64> {
    managed_usage_at(blob_dir)
}

pub fn available_space(path: &Path) -> anyhow::Result<u64> {
    available_space_at(path)
}

pub(crate) fn install_staged_locked(
    blob_dir: &Path,
    stage_root: &Path,
    staged: StagedImage,
) -> anyhow::Result<InstalledImage> {
    let stage_dir = validate_stage_directory(stage_root, &staged)?;
    let mut created_paths = Vec::with_capacity(2);
    let install_result = (|| {
        validate_stage_files(&stage_dir, &staged)?;
        let expected_thumbnail =
            decoded_thumbnail_for_hash(staged.bmp_path(), staged.content_hash())?;
        validate_thumbnail(staged.thumbnail_path(), &expected_thumbnail)?;
        let bmp_path = canonical_bmp_path(blob_dir, staged.content_hash())?;
        let thumbnail_path = canonical_thumbnail_path(blob_dir, staged.content_hash())?;

        install_one(
            staged.bmp_path(),
            &bmp_path,
            &mut created_paths,
            |existing| decoded_thumbnail_for_hash(existing, staged.content_hash()).map(|_| ()),
        )?;
        install_one(
            staged.thumbnail_path(),
            &thumbnail_path,
            &mut created_paths,
            |existing| validate_thumbnail(existing, &expected_thumbnail),
        )?;
        fs::remove_file(staged.bmp_path())
            .with_context(|| format!("remove installed stage {}", staged.bmp_path().display()))?;
        fs::remove_file(staged.thumbnail_path()).with_context(|| {
            format!(
                "remove installed stage {}",
                staged.thumbnail_path().display()
            )
        })?;
        Ok((bmp_path, thumbnail_path))
    })();

    let (bmp_path, thumbnail_path) = match install_result {
        Ok(paths) => paths,
        Err(install_error) => {
            return Err(clean_failed_install(
                &stage_dir,
                &created_paths,
                install_error,
            ));
        }
    };

    if let Err(error) = fs::remove_dir(&stage_dir) {
        crate::diagnostics::warn(format!(
            "blobs: failed to remove empty image stage {}: {}",
            stage_dir.display(),
            error
        ));
    }

    Ok(InstalledImage {
        content_hash: staged.content_hash().to_string(),
        bmp_path,
        thumbnail_path,
        created_paths,
    })
}

fn validate_stage_directory(stage_root: &Path, staged: &StagedImage) -> anyhow::Result<PathBuf> {
    let stage_root = stage_root
        .canonicalize()
        .with_context(|| format!("canonicalize stage root {}", stage_root.display()))?;
    let stage_dir = staged
        .stage_dir()
        .canonicalize()
        .with_context(|| format!("canonicalize image stage {}", staged.stage_dir().display()))?;
    ensure!(
        stage_dir.parent() == Some(stage_root.as_path())
            && staged.stage_dir() == stage_dir.as_path(),
        "staged image is outside managed stage root"
    );
    Ok(stage_dir)
}

fn validate_stage_files(stage_dir: &Path, staged: &StagedImage) -> anyhow::Result<()> {
    ensure!(
        fs::symlink_metadata(staged.bmp_path())?
            .file_type()
            .is_file()
            && fs::symlink_metadata(staged.thumbnail_path())?
                .file_type()
                .is_file(),
        "staged image paths must be regular files"
    );
    ensure!(
        staged.bmp_path() == canonical_bmp_path(stage_dir, staged.content_hash())?
            && staged.thumbnail_path()
                == canonical_thumbnail_path(stage_dir, staged.content_hash())?,
        "staged image paths are not canonical"
    );
    Ok(())
}
fn install_one(
    staged_path: &Path,
    canonical_path: &Path,
    created_paths: &mut Vec<PathBuf>,
    validate_existing: impl FnOnce(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    match fs::hard_link(staged_path, canonical_path) {
        Ok(()) => {
            created_paths.push(canonical_path.to_path_buf());
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_existing(canonical_path).with_context(|| {
                format!(
                    "validate existing canonical image {}",
                    canonical_path.display()
                )
            })
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "atomically install staged image {} as {}",
                staged_path.display(),
                canonical_path.display()
            )
        }),
    }
}

fn clean_failed_install(
    stage_dir: &Path,
    created_paths: &[PathBuf],
    install_error: anyhow::Error,
) -> anyhow::Error {
    let mut cleanup_errors = Vec::new();
    if let Err(error) = rollback_paths(created_paths) {
        cleanup_errors.push(format!("rollback canonical files: {error:#}"));
    }
    if let Err(error) = fs::remove_dir_all(stage_dir) {
        cleanup_errors.push(format!("remove stage {}: {error}", stage_dir.display()));
    }
    if cleanup_errors.is_empty() {
        install_error
    } else {
        anyhow!(
            "{install_error:#}; additionally failed cleanup: {}",
            cleanup_errors.join("; ")
        )
    }
}

fn clean_after_persist_failure(
    installed: &InstalledImage,
    persist_error: anyhow::Error,
    is_referenced: impl FnOnce(&InstalledImage) -> anyhow::Result<bool>,
) -> anyhow::Error {
    match is_referenced(installed) {
        Ok(true) => persist_error,
        Ok(false) => match rollback_paths(&installed.created_paths) {
            Ok(()) => persist_error,
            Err(error) => {
                anyhow!("{persist_error:#}; additionally failed safe rollback: {error:#}")
            }
        },
        Err(error) => {
            anyhow!("{persist_error:#}; reference check failed and files were retained: {error:#}")
        }
    }
}
fn rollback_paths(paths: &[PathBuf]) -> anyhow::Result<()> {
    let mut first_error = None;
    for path in paths.iter().rev() {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound && first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error.into()),
        None => Ok(()),
    }
}

fn managed_usage_at(root: &Path) -> anyhow::Result<u64> {
    let mut usage = 0u64;
    for entry in fs::read_dir(root)
        .with_context(|| format!("read managed blob directory {}", root.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            usage = usage
                .checked_add(managed_usage_at(&entry.path())?)
                .ok_or_else(|| anyhow!("managed blob usage overflow"))?;
        } else if file_type.is_file() {
            usage = usage
                .checked_add(entry.metadata()?.len())
                .ok_or_else(|| anyhow!("managed blob usage overflow"))?;
        }
    }
    Ok(usage)
}

#[cfg(target_os = "windows")]
fn available_space_at(path: &Path) -> anyhow::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    let success = unsafe {
        GetDiskFreeSpaceExW(
            wide_path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("query available disk space for {}", path.display()));
    }
    Ok(available)
}

#[cfg(not(target_os = "windows"))]
fn available_space_at(path: &Path) -> anyhow::Result<u64> {
    Err(anyhow!(
        "available-space query is unsupported on this platform for {}",
        path.display()
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::blobs::image::stage_dib;
    use ::image::{ImageBuffer, Rgba};
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;
    use uuid::Uuid;

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("super-clipboard-{label}-{}", Uuid::new_v4()))
    }

    fn dib32(bgra: [u8; 4]) -> Vec<u8> {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[20..24].copy_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&bgra);
        dib
    }

    fn new_store(label: &str) -> (PathBuf, ImageBlobStore) {
        let root = temp_dir(label);
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        (root, store)
    }

    fn install_ok(store: &ImageBlobStore, staged: StagedImage) -> InstalledImage {
        store
            .install_staged_with(staged, |installed| Ok(installed.clone()), |_| Ok(false))
            .expect("install")
    }

    #[test]
    fn rejects_blob_and_stage_directories_that_are_not_siblings() {
        let root = temp_dir("non-sibling");

        assert!(ImageBlobStore::new(root.join("one/blobs"), root.join("two/stage")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn write_lease_waits_for_read_lease() {
        let (root, store) = new_store("lease");
        let store = Arc::new(store);
        let (read_entered_tx, read_entered_rx) = mpsc::channel();
        let (release_read_tx, release_read_rx) = mpsc::channel();
        let (write_entered_tx, write_entered_rx) = mpsc::channel();

        let reader_store = Arc::clone(&store);
        let reader = thread::spawn(move || {
            reader_store.with_read(|_| {
                read_entered_tx.send(()).expect("signal reader");
                release_read_rx.recv().expect("release reader");
                Ok(())
            })
        });
        read_entered_rx.recv().expect("reader entered");

        let writer_store = Arc::clone(&store);
        let writer = thread::spawn(move || {
            writer_store.with_write(|_, _| {
                write_entered_tx.send(()).expect("signal writer");
                Ok(())
            })
        });

        assert!(write_entered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_read_tx.send(()).expect("release");
        write_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer entered after reader");
        reader
            .join()
            .expect("reader thread")
            .expect("reader result");
        writer
            .join()
            .expect("writer thread")
            .expect("writer result");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn valid_canonical_hit_is_verified_and_reused() {
        let (root, store) = new_store("canonical-hit");
        let first = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("first stage");
        let first_install = install_ok(&store, first);
        let second = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("second stage");
        let second_stage_dir = second.stage_dir().to_path_buf();

        let second_install = install_ok(&store, second);

        assert_eq!(second_install.content_hash(), first_install.content_hash());
        assert!(second_install.created_paths().is_empty());
        assert_eq!(second_install.bmp_path(), first_install.bmp_path());
        assert!(!second_stage_dir.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn corrupt_staged_thumbnail_is_rejected_and_stage_is_cleaned() {
        let (root, store) = new_store("corrupt-stage");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let stage_dir = staged.stage_dir().to_path_buf();
        fs::write(staged.thumbnail_path(), b"corrupt staged thumbnail").expect("tamper stage");

        assert!(store
            .install_staged_with(staged, |_| Ok(()), |_| Ok(false))
            .is_err());
        assert!(!stage_dir.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn semantic_mismatch_existing_bmp_is_rejected_without_overwrite() {
        let (root, store) = new_store("wrong-bmp");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let stage_dir = staged.stage_dir().to_path_buf();
        let canonical_bmp =
            canonical_bmp_path(store.blob_dir(), staged.content_hash()).expect("path");
        let wrong = stage_dib(store.stage_dir(), dib32([31, 20, 10, 255])).expect("wrong stage");
        let wrong_bytes = fs::read(wrong.bmp_path()).expect("wrong BMP bytes");
        fs::write(&canonical_bmp, &wrong_bytes).expect("valid wrong BMP");
        fs::remove_dir_all(wrong.stage_dir()).expect("remove wrong stage");

        assert!(store
            .install_staged_with(staged, |_| Ok(()), |_| Ok(false))
            .is_err());
        assert_eq!(
            fs::read(canonical_bmp).expect("preserved wrong BMP"),
            wrong_bytes
        );
        assert!(!stage_dir.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn mismatched_decodable_thumbnail_is_rejected_without_overwrite() {
        let (root, store) = new_store("wrong-thumbnail");
        let first = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("first stage");
        let installed = install_ok(&store, first);
        let wrong_thumbnail = ImageBuffer::from_pixel(1, 1, Rgba([250u8, 1, 2, 255]));
        wrong_thumbnail
            .save(installed.thumbnail_path())
            .expect("write wrong thumbnail");
        let wrong_bytes = fs::read(installed.thumbnail_path()).expect("wrong thumbnail bytes");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("second stage");
        let stage_dir = staged.stage_dir().to_path_buf();

        assert!(store
            .install_staged_with(staged, |_| Ok(()), |_| Ok(false))
            .is_err());
        assert_eq!(
            fs::read(installed.thumbnail_path()).expect("preserved wrong thumbnail"),
            wrong_bytes
        );
        assert!(!stage_dir.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn second_file_install_failure_rolls_back_new_bmp_and_cleans_stage() {
        let (root, store) = new_store("partial-install");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let stage_dir = staged.stage_dir().to_path_buf();
        let bmp_path =
            canonical_bmp_path(store.blob_dir(), staged.content_hash()).expect("BMP path");
        let thumbnail_path = canonical_thumbnail_path(store.blob_dir(), staged.content_hash())
            .expect("thumbnail path");
        fs::create_dir(&thumbnail_path).expect("blocking thumbnail directory");
        assert!(!bmp_path.exists());

        assert!(store
            .install_staged_with(staged, |_| Ok(()), |_| Ok(false))
            .is_err());

        assert!(!bmp_path.exists(), "new BMP must be rolled back");
        assert!(thumbnail_path.is_dir());
        assert!(!stage_dir.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn write_lease_covers_persist_and_reference_check_callbacks() {
        let (root, store) = new_store("transaction-lease");
        let store = Arc::new(store);
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let (writer_entered_tx, writer_entered_rx) = mpsc::channel();
        let writer_store = Arc::clone(&store);
        let mut writer = None;

        let result = store.install_staged_with(
            staged,
            |_| {
                writer = Some(thread::spawn(move || {
                    writer_store.with_write(|_, _| {
                        writer_entered_tx.send(()).expect("signal writer");
                        Ok(())
                    })
                }));
                assert!(writer_entered_rx
                    .recv_timeout(Duration::from_millis(100))
                    .is_err());
                Err::<(), _>(anyhow!("persist failed"))
            },
            |_| {
                assert!(writer_entered_rx
                    .recv_timeout(Duration::from_millis(100))
                    .is_err());
                Ok(true)
            },
        );

        assert!(result.is_err());
        writer_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer entered after transaction");
        writer
            .expect("writer handle")
            .join()
            .expect("writer thread")
            .expect("writer result");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn persist_failure_keeps_new_files_when_reference_exists() {
        use std::cell::Cell;

        let (root, store) = new_store("referenced-rollback");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let bmp_path =
            canonical_bmp_path(store.blob_dir(), staged.content_hash()).expect("BMP path");
        let thumbnail_path = canonical_thumbnail_path(store.blob_dir(), staged.content_hash())
            .expect("thumbnail path");
        let referenced = Cell::new(false);

        let result = store.install_staged_with(
            staged,
            |installed| {
                assert!(installed.bmp_path().is_file());
                assert!(installed.thumbnail_path().is_file());
                referenced.set(true);
                Err::<(), _>(anyhow!("persist failed after competing reference"))
            },
            |_| Ok(referenced.get()),
        );

        assert!(result.is_err());
        assert!(bmp_path.is_file());
        assert!(thumbnail_path.is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn persist_failure_removes_only_new_unreferenced_files() {
        let (root, store) = new_store("unreferenced-rollback");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let bmp_path =
            canonical_bmp_path(store.blob_dir(), staged.content_hash()).expect("BMP path");
        let thumbnail_path = canonical_thumbnail_path(store.blob_dir(), staged.content_hash())
            .expect("thumbnail path");

        let result = store.install_staged_with(
            staged,
            |_| Err::<(), _>(anyhow!("persist failed")),
            |_| Ok(false),
        );

        assert!(result.is_err());
        assert!(!bmp_path.exists());
        assert!(!thumbnail_path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reference_check_failure_keeps_orphans() {
        let (root, store) = new_store("reference-check-error");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let bmp_path =
            canonical_bmp_path(store.blob_dir(), staged.content_hash()).expect("BMP path");
        let thumbnail_path = canonical_thumbnail_path(store.blob_dir(), staged.content_hash())
            .expect("thumbnail path");

        let result = store.install_staged_with(
            staged,
            |_| Err::<(), _>(anyhow!("persist failed")),
            |_| Err(anyhow!("reference query failed")),
        );

        assert!(result.is_err());
        assert!(bmp_path.is_file());
        assert!(thumbnail_path.is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn managed_usage_counts_regular_files_recursively() {
        let (root, store) = new_store("usage");
        fs::write(store.blob_dir().join("one.bmp"), [0u8; 7]).expect("first file");
        fs::create_dir_all(store.blob_dir().join("nested")).expect("nested dir");
        fs::write(store.blob_dir().join("nested/two.tmp"), [0u8; 11]).expect("second file");

        assert_eq!(store.managed_usage().expect("usage"), 18);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reports_free_space_for_the_blob_volume() {
        let (root, store) = new_store("free-space");

        assert!(store.available_space().expect("available space") > 0);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
