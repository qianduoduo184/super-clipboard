use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{anyhow, Context};

use super::image::{canonical_bmp_path, canonical_thumbnail_path, StagedImage};

#[derive(Debug)]
pub struct ImageBlobStore {
    blob_dir: PathBuf,
    stage_dir: PathBuf,
    lease: RwLock<()>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledImage {
    pub content_hash: String,
    pub bmp_path: PathBuf,
    pub thumbnail_path: PathBuf,
    pub created_paths: Vec<PathBuf>,
}

impl ImageBlobStore {
    pub fn new(blob_dir: PathBuf, stage_dir: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&blob_dir)
            .with_context(|| format!("create image blob directory {}", blob_dir.display()))?;
        fs::create_dir_all(&stage_dir)
            .with_context(|| format!("create image stage directory {}", stage_dir.display()))?;
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
        let _lease = self
            .lease
            .read()
            .map_err(|_| anyhow!("image blob read lease is poisoned"))?;
        f(&self.blob_dir)
    }

    pub fn with_write<T>(
        &self,
        f: impl FnOnce(&Path, &Path) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let _lease = self
            .lease
            .write()
            .map_err(|_| anyhow!("image blob write lease is poisoned"))?;
        f(&self.blob_dir, &self.stage_dir)
    }

    pub fn managed_usage(&self) -> anyhow::Result<u64> {
        self.with_read(managed_usage)
    }

    pub fn available_space(&self) -> anyhow::Result<u64> {
        self.with_read(available_space)
    }
}

pub fn managed_usage(blob_dir: &Path) -> anyhow::Result<u64> {
    managed_usage_at(blob_dir)
}

pub fn available_space(path: &Path) -> anyhow::Result<u64> {
    available_space_at(path)
}

pub fn install_staged(blob_dir: &Path, staged: StagedImage) -> anyhow::Result<InstalledImage> {
    fs::create_dir_all(blob_dir)
        .with_context(|| format!("create image blob directory {}", blob_dir.display()))?;
    let bmp_path = canonical_bmp_path(blob_dir, &staged.content_hash);
    let thumbnail_path = canonical_thumbnail_path(blob_dir, &staged.content_hash);
    let mut created_paths = Vec::with_capacity(2);

    let install_result = (|| {
        install_one(&staged.bmp_path, &bmp_path, &mut created_paths)?;
        install_one(&staged.thumbnail_path, &thumbnail_path, &mut created_paths)?;
        Ok(())
    })();

    if let Err(error) = install_result {
        rollback_paths(&created_paths)
            .with_context(|| format!("roll back partial image install after: {error:#}"))?;
        return Err(error);
    }

    if let Some(stage_parent) = staged.bmp_path.parent() {
        let _ = fs::remove_dir(stage_parent);
    }

    Ok(InstalledImage {
        content_hash: staged.content_hash,
        bmp_path,
        thumbnail_path,
        created_paths,
    })
}

pub fn rollback_install(installed: &InstalledImage) -> anyhow::Result<()> {
    rollback_paths(&installed.created_paths)
}

fn install_one(
    staged_path: &Path,
    canonical_path: &Path,
    created_paths: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    if canonical_path.exists() {
        if !canonical_path.is_file() {
            return Err(anyhow!(
                "canonical blob path is not a file: {}",
                canonical_path.display()
            ));
        }
        fs::remove_file(staged_path)
            .with_context(|| format!("remove redundant stage {}", staged_path.display()))?;
        return Ok(());
    }

    fs::rename(staged_path, canonical_path).with_context(|| {
        format!(
            "install staged image {} as {}",
            staged_path.display(),
            canonical_path.display()
        )
    })?;
    created_paths.push(canonical_path.to_path_buf());
    Ok(())
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
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;
    use uuid::Uuid;

    fn temp_dir(label: &str) -> std::path::PathBuf {
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

    #[test]
    fn write_lease_waits_for_read_lease() {
        let root = temp_dir("lease");
        let store =
            Arc::new(ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store"));
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
    fn rollback_removes_only_files_created_by_this_install() {
        let root = temp_dir("rollback");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let staged = stage_dib(store.stage_dir(), dib32([30, 20, 10, 255])).expect("stage");
        let canonical_bmp = store
            .blob_dir()
            .join(format!("{}.bmp", staged.content_hash));
        fs::write(&canonical_bmp, b"existing").expect("existing bmp");

        let installed = store
            .with_write(|blob_dir, _| install_staged(blob_dir, staged))
            .expect("install");
        assert_eq!(
            fs::read(&canonical_bmp).expect("preserved bmp"),
            b"existing"
        );
        assert!(installed.thumbnail_path.is_file());

        rollback_install(&installed).expect("rollback");

        assert_eq!(
            fs::read(&canonical_bmp).expect("preserved bmp"),
            b"existing"
        );
        assert!(!installed.thumbnail_path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn managed_usage_counts_regular_files_recursively() {
        let root = temp_dir("usage");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        fs::write(store.blob_dir().join("one.bmp"), [0u8; 7]).expect("first file");
        fs::create_dir_all(store.blob_dir().join("nested")).expect("nested dir");
        fs::write(store.blob_dir().join("nested/two.tmp"), [0u8; 11]).expect("second file");

        assert_eq!(store.managed_usage().expect("usage"), 18);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reports_free_space_for_the_blob_volume() {
        let root = temp_dir("free-space");
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");

        assert!(store.available_space().expect("available space") > 0);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
