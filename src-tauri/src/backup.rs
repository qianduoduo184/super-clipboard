use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::blobs::store::ImageBlobStore;
use crate::storage::repository::{ClipboardItem, ClipboardRepository};

#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    version: u32,
    exported_at: String,
    item_count: usize,
    items: Vec<ClipboardItem>,
}

const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LEGACY_PATH_BYTES: usize = 1024;
// Base64 may expand the 5 GiB managed blob quota by 4/3. Reserve one more
// managed quota for item text and JSON structure without imposing a 64 MiB cap.
const MAX_LEGACY_INFO_BYTES: u64 = {
    let base64_groups = match crate::storage::capacity::MANAGED_BLOB_QUOTA.checked_add(2) {
        Some(bytes) => bytes / 3,
        None => panic!("legacy backup size overflow"),
    };
    let base64_bytes = match base64_groups.checked_mul(4) {
        Some(bytes) => bytes,
        None => panic!("legacy backup size overflow"),
    };
    match base64_bytes.checked_add(crate::storage::capacity::MANAGED_BLOB_QUOTA) {
        Some(bytes) => bytes,
        None => panic!("legacy backup size overflow"),
    }
};

struct BoundedString<const MAX_BYTES: usize>(String);

struct BoundedStringVisitor<const MAX_BYTES: usize>;

impl<const MAX_BYTES: usize> Visitor<'_> for BoundedStringVisitor<MAX_BYTES> {
    type Value = BoundedString<MAX_BYTES>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "a string no longer than {MAX_BYTES} bytes")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if value.len() > MAX_BYTES {
            return Err(E::custom(format!(
                "string exceeds the {MAX_BYTES}-byte limit"
            )));
        }
        Ok(BoundedString(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if value.len() > MAX_BYTES {
            return Err(E::custom(format!(
                "string exceeds the {MAX_BYTES}-byte limit"
            )));
        }
        Ok(BoundedString(value))
    }
}

impl<'de, const MAX_BYTES: usize> Deserialize<'de> for BoundedString<MAX_BYTES> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(BoundedStringVisitor::<MAX_BYTES>)
    }
}

#[derive(Deserialize)]
struct LegacyMetadataInfo {
    version: BoundedString<32>,
    created_at: BoundedString<128>,
    item_count: usize,
}

#[derive(Deserialize)]
struct LegacyItemInfo {
    item_type: BoundedString<32>,
    content_path: Option<BoundedString<MAX_LEGACY_PATH_BYTES>>,
    #[serde(default, rename = "content")]
    _content: Option<IgnoredAny>,
}

struct LegacyItems(usize);

impl<'de> Deserialize<'de> for LegacyItems {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LegacyItemsVisitor;

        impl<'de> Visitor<'de> for LegacyItemsVisitor {
            type Value = LegacyItems;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a legacy backup item array")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut count = 0usize;
                while let Some(item) = sequence.next_element::<LegacyItemInfo>()? {
                    if item.item_type.0 == "image" {
                        let path = item.content_path.ok_or_else(|| {
                            <A::Error as serde::de::Error>::custom(
                                "legacy image item is missing content_path",
                            )
                        })?;
                        validate_legacy_bmp_filename(&path.0)
                            .map_err(<A::Error as serde::de::Error>::custom)?;
                    }
                    count = count.checked_add(1).ok_or_else(|| {
                        <A::Error as serde::de::Error>::custom("legacy item count overflow")
                    })?;
                }
                Ok(LegacyItems(count))
            }
        }

        deserializer.deserialize_seq(LegacyItemsVisitor)
    }
}

#[derive(Deserialize)]
struct LegacyBlobInfo {
    filename: BoundedString<MAX_LEGACY_PATH_BYTES>,
    #[serde(default, rename = "data_base64")]
    _data_base64: Option<IgnoredAny>,
}

struct LegacyBlobs;

impl<'de> Deserialize<'de> for LegacyBlobs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LegacyBlobsVisitor;

        impl<'de> Visitor<'de> for LegacyBlobsVisitor {
            type Value = LegacyBlobs;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a legacy backup blob array")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut aliases = HashSet::new();
                while let Some(blob) = sequence.next_element::<LegacyBlobInfo>()? {
                    validate_legacy_bmp_filename(&blob.filename.0)
                        .map_err(<A::Error as serde::de::Error>::custom)?;
                    if !aliases.insert(blob.filename.0.to_ascii_lowercase()) {
                        return Err(<A::Error as serde::de::Error>::custom(
                            "legacy backup contains aliased blob filenames",
                        ));
                    }
                }
                Ok(LegacyBlobs)
            }
        }

        deserializer.deserialize_seq(LegacyBlobsVisitor)
    }
}

#[derive(Deserialize)]
struct LegacyBackupInfo {
    metadata: LegacyMetadataInfo,
    items: LegacyItems,
    #[serde(rename = "blobs")]
    _blobs: LegacyBlobs,
}

fn validate_legacy_bmp_filename(filename: &str) -> Result<(), String> {
    let path = Path::new(filename);
    let mut components = path.components();
    let is_single_component = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    );
    if path.is_absolute()
        || !is_single_component
        || filename.contains(['/', '\\'])
        || filename.ends_with(['.', ' '])
        || filename
            .chars()
            .any(|character| character < ' ' || r#"<>:\"|?*"#.contains(character))
    {
        return Err("legacy blob path must be a safe single filename".to_string());
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    let is_windows_device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
    if is_windows_device {
        return Err("legacy blob filename must not use a Windows device alias".to_string());
    }
    let is_bmp = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bmp"));
    if !is_bmp {
        return Err("legacy blob filename must use the .bmp extension".to_string());
    }
    Ok(())
}

fn parse_legacy_info(file: File, path: &Path) -> anyhow::Result<crate::commands::BackupInfo> {
    let file_len = file
        .metadata()
        .with_context(|| format!("inspect legacy backup {}", path.display()))?
        .len();
    anyhow::ensure!(
        file_len <= MAX_LEGACY_INFO_BYTES,
        "legacy backup exceeds the quota-derived metadata preview limit"
    );

    let mut deserializer = serde_json::Deserializer::from_reader(file);
    let backup = LegacyBackupInfo::deserialize(&mut deserializer)
        .with_context(|| format!("parse legacy backup {}", path.display()))?;
    deserializer
        .end()
        .with_context(|| format!("parse legacy backup {}", path.display()))?;
    anyhow::ensure!(
        backup.metadata.version.0 == "1.0",
        "unsupported legacy backup version"
    );
    anyhow::ensure!(
        backup.metadata.item_count == backup.items.0,
        "legacy backup item_count does not match items"
    );
    Ok(crate::commands::BackupInfo {
        created_at: backup.metadata.created_at.0,
        item_count: backup.metadata.item_count,
        version: backup.metadata.version.0,
    })
}

pub fn parse_backup_info_path(path: &Path) -> anyhow::Result<crate::commands::BackupInfo> {
    let mut file = File::open(path).with_context(|| format!("open backup {}", path.display()))?;
    let mut magic = [0u8; 4];
    let count = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    if count == magic.len() && is_zip_magic(magic) {
        parse_zip_info(file)
    } else {
        parse_legacy_info(file, path)
    }
}

pub(crate) fn is_zip_magic(magic: [u8; 4]) -> bool {
    matches!(
        magic,
        [b'P', b'K', 3, 4] | [b'P', b'K', 5, 6] | [b'P', b'K', 7, 8]
    )
}

fn parse_zip_info(file: File) -> anyhow::Result<crate::commands::BackupInfo> {
    let mut archive = zip::ZipArchive::new(file).context("parse ZIP backup")?;
    anyhow::ensure!(!archive.is_empty(), "ZIP backup is empty");
    let first_name = archive.by_index(0)?.name().to_string();
    anyhow::ensure!(
        first_name == "manifest.json",
        "manifest.json must be the first ZIP entry"
    );
    let mut entry_names = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        anyhow::ensure!(!entry.is_dir(), "backup ZIP must not contain directories");
        anyhow::ensure!(
            name == "manifest.json" || archive_blob_hash(&name).is_some(),
            "invalid backup ZIP entry path: {name}"
        );
        anyhow::ensure!(
            entry_names.insert(name.clone()),
            "duplicate ZIP entry: {name}"
        );
    }
    let manifest: BackupManifest = {
        let manifest_file = archive.by_name("manifest.json")?;
        anyhow::ensure!(
            manifest_file.size() <= MAX_MANIFEST_BYTES,
            "backup manifest exceeds {} bytes",
            MAX_MANIFEST_BYTES
        );
        let mut limited = manifest_file.take(MAX_MANIFEST_BYTES + 1);
        let mut manifest_bytes = Vec::new();
        limited.read_to_end(&mut manifest_bytes)?;
        anyhow::ensure!(
            manifest_bytes.len() as u64 <= MAX_MANIFEST_BYTES,
            "backup manifest exceeds {} bytes",
            MAX_MANIFEST_BYTES
        );
        serde_json::from_slice(&manifest_bytes).context("parse backup manifest")?
    };
    anyhow::ensure!(manifest.version == 2, "unsupported backup manifest version");
    anyhow::ensure!(
        manifest.item_count == manifest.items.len(),
        "backup manifest item_count does not match items"
    );
    anyhow::ensure!(
        !manifest.exported_at.is_empty(),
        "backup manifest exported_at is empty"
    );
    for item in &manifest.items {
        if item.item_type != "image" {
            continue;
        }
        let content_hash = item
            .content_hash
            .as_deref()
            .filter(|value| is_valid_content_hash(value))
            .ok_or_else(|| anyhow::anyhow!("image item {} has invalid content_hash", item.id))?;
        let expected_path = format!("blobs/{content_hash}.bmp");
        anyhow::ensure!(
            item.content_path.as_deref() == Some(expected_path.as_str()),
            "image item {} has invalid archive path",
            item.id
        );
        anyhow::ensure!(
            entry_names.contains(&expected_path),
            "image item {} references a missing blob",
            item.id
        );
    }
    Ok(crate::commands::BackupInfo {
        created_at: manifest.exported_at,
        item_count: manifest.item_count,
        version: manifest.version.to_string(),
    })
}

fn archive_blob_hash(name: &str) -> Option<&str> {
    let hash = name.strip_prefix("blobs/")?.strip_suffix(".bmp")?;
    is_valid_content_hash(hash).then_some(hash)
}

fn is_valid_content_hash(content_hash: &str) -> bool {
    content_hash.len() == 64
        && content_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

fn write_blob_entry<W: Write + Seek, R: Read>(
    archive: &mut ZipWriter<W>,
    archive_path: &str,
    reader: &mut R,
) -> anyhow::Result<u64> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    archive.start_file(archive_path, options)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut written = 0u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        archive.write_all(&buffer[..count])?;
        written += count as u64;
    }
    Ok(written)
}

struct TempBackup {
    path: std::path::PathBuf,
    keep: bool,
}

impl Drop for TempBackup {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn write_backup_atomically(
    target: &Path,
    write: impl FnOnce(&mut File) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("backup target must have a file name"))?;
    let parent = target
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .with_context(|| format!("canonicalize backup directory for {}", target.display()))?;
    anyhow::ensure!(parent.is_dir(), "backup target parent is not a directory");
    let target = parent.join(file_name);
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        anyhow::ensure!(
            metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
            "backup target must be a regular file"
        );
    }
    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let mut temp = TempBackup {
        path: temp_path.clone(),
        keep: false,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .with_context(|| format!("create temporary backup {}", temp_path.display()))?;
    write(&mut file)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    replace_target(&temp_path, &target)?;
    temp.keep = true;
    Ok(())
}

#[cfg(target_os = "windows")]
fn replace_target(temp_path: &Path, target: &Path) -> anyhow::Result<()> {
    if !target.exists() {
        return fs::rename(temp_path, target).with_context(|| {
            format!(
                "install backup {} as {}",
                temp_path.display(),
                target.display()
            )
        });
    }
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let temp_wide = temp_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            temp_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "replace backup {} with {}",
                target.display(),
                temp_path.display()
            )
        });
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_target(temp_path: &Path, target: &Path) -> anyhow::Result<()> {
    fs::rename(temp_path, target).with_context(|| {
        format!(
            "install backup {} as {}",
            temp_path.display(),
            target.display()
        )
    })
}

pub fn export_zip_to(
    path: &Path,
    repository: &Mutex<ClipboardRepository>,
    blob_store: &ImageBlobStore,
) -> anyhow::Result<()> {
    export_zip_to_with_hook(path, repository, blob_store, || Ok(()))
}

fn export_zip_to_with_hook(
    path: &Path,
    repository: &Mutex<ClipboardRepository>,
    blob_store: &ImageBlobStore,
    hook: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let items = repository
        .lock()
        .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
        .list_items_for_backup(100_000)?;
    export_snapshot_to_with_hook(path, items, blob_store, hook)
}

fn export_snapshot_to_with_hook(
    path: &Path,
    mut items: Vec<ClipboardItem>,
    blob_store: &ImageBlobStore,
    hook: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    blob_store.with_read(|blob_dir| {
        let mut seen_hashes = HashSet::new();
        let mut blobs = Vec::new();
        for item in &mut items {
            if item.item_type != "image" {
                continue;
            }
            let content_hash = item
                .content_hash
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("image item {} is missing content_hash", item.id))?;
            let expected_path = crate::blobs::image::canonical_bmp_path(blob_dir, content_hash)?;
            let source_path = item
                .content_path
                .as_deref()
                .map(Path::new)
                .map(Path::to_path_buf)
                .ok_or_else(|| anyhow::anyhow!("image item {} is missing content_path", item.id))?;
            anyhow::ensure!(
                source_path.is_absolute() && source_path == expected_path,
                "image item {} path is outside the canonical blob location",
                item.id
            );
            let path_metadata = fs::symlink_metadata(&source_path)
                .with_context(|| format!("inspect image blob {}", source_path.display()))?;
            anyhow::ensure!(
                path_metadata.file_type().is_file()
                    && !path_metadata.file_type().is_symlink()
                    && !is_reparse_point(&path_metadata),
                "image blob must be a regular file"
            );
            let canonical_source = source_path
                .canonicalize()
                .with_context(|| format!("canonicalize image blob {}", source_path.display()))?;
            anyhow::ensure!(
                canonical_source == expected_path,
                "image blob resolves outside the canonical blob location"
            );
            let archive_path = format!("blobs/{content_hash}.bmp");
            item.content_path = Some(archive_path.clone());
            if seen_hashes.insert(content_hash.to_string()) {
                let mut file = File::open(&source_path)
                    .with_context(|| format!("open image blob {}", source_path.display()))?;
                let metadata = file.metadata().with_context(|| {
                    format!("inspect open image blob {}", source_path.display())
                })?;
                anyhow::ensure!(
                    metadata.is_file(),
                    "image blob handle must be a regular file"
                );
                crate::blobs::validate_bmp_file_header(&mut file, metadata.len())
                    .with_context(|| format!("validate image blob {}", source_path.display()))?;
                blobs.push((archive_path, source_path, file));
            }
        }
        hook()?;

        let manifest = BackupManifest {
            version: 2,
            exported_at: chrono::Utc::now().to_rfc3339(),
            item_count: items.len(),
            items,
        };
        write_backup_atomically(path, |file| {
            let mut archive = ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            archive.start_file("manifest.json", options)?;
            serde_json::to_writer(&mut archive, &manifest)?;
            for (archive_path, source_path, mut file) in blobs {
                file.seek(SeekFrom::Start(0))
                    .with_context(|| format!("rewind image blob {}", source_path.display()))?;
                write_blob_entry(&mut archive, &archive_path, &mut file)
                    .with_context(|| format!("archive image blob {}", source_path.display()))?;
            }
            archive.finish()?;
            Ok(())
        })
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Cursor, Read, Write};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};
    use crate::storage::repository::{ClipboardItem, ClipboardRepository};

    use super::{
        export_snapshot_to_with_hook, export_zip_to, export_zip_to_with_hook,
        parse_backup_info_path, write_backup_atomically, write_blob_entry, MAX_MANIFEST_BYTES,
    };

    fn temp_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "super-clipboard-backup-{label}-{}",
            uuid::Uuid::new_v4()
        ))
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

    fn opaque_bmp(payload_len: usize) -> Vec<u8> {
        let file_len = 14usize.checked_add(payload_len).expect("BMP length");
        let mut bmp = vec![0u8; file_len];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(
            &u32::try_from(file_len)
                .expect("test BMP fits u32")
                .to_le_bytes(),
        );
        bmp[10..14].copy_from_slice(&14u32.to_le_bytes());
        bmp
    }

    fn write_test_zip(path: &std::path::Path, manifest: &serde_json::Value, entries: &[&str]) {
        let file = fs::File::create(path).expect("create test zip");
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file("manifest.json", options)
            .expect("start manifest");
        serde_json::to_writer(&mut archive, manifest).expect("write manifest");
        for name in entries {
            archive.start_file(*name, options).expect("start entry");
            archive.write_all(b"blob").expect("write entry");
        }
        archive.finish().expect("finish test zip");
    }

    fn manifest_image(path: &str, content_hash: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "image",
            "hash": "record-hash",
            "item_type": "image",
            "content": null,
            "content_path": path,
            "content_hash": content_hash,
            "preview": "image",
            "source_app": null,
            "favorite": false,
            "pinned": false,
            "size_bytes": 4,
            "created_at": 1,
            "updated_at": 1
        })
    }

    fn legacy_item(item_type: &str, content_path: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "id": "legacy-item",
            "hash": "legacy-record-hash",
            "item_type": item_type,
            "content": null,
            "content_path": content_path,
            "preview": "legacy preview",
            "source_app": null,
            "favorite": false,
            "pinned": false,
            "size_bytes": 4,
            "created_at": 1,
            "updated_at": 1
        })
    }

    fn legacy_blob(filename: &str, data_base64: &str) -> serde_json::Value {
        serde_json::json!({
            "item_id": "legacy-item",
            "filename": filename,
            "data_base64": data_base64
        })
    }

    fn write_legacy_json(
        path: &std::path::Path,
        version: &str,
        item_count: usize,
        items: Vec<serde_json::Value>,
        blobs: Vec<serde_json::Value>,
    ) {
        fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({
                "metadata": {
                    "version": version,
                    "created_at": "2026-07-15T00:00:00Z",
                    "item_count": item_count
                },
                "items": items,
                "blobs": blobs
            }))
            .expect("legacy JSON"),
        )
        .expect("write legacy JSON");
    }

    #[test]
    fn export_manifest_v2_preserves_full_non_image_content() {
        let root = temp_dir("manifest");
        fs::create_dir_all(&root).expect("root");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        repository
            .lock()
            .expect("repository lock")
            .insert_or_touch(ClipboardItemDraft {
                item_type: ClipboardItemType::Text,
                content: Some("完整正文\n第二行".to_string()),
                content_path: None,
                content_hash: None,
                preview: "完整正文...".to_string(),
                source_app: Some("fixture".to_string()),
                size_bytes: 24,
            })
            .expect("insert text");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let archive_path = root.join("backup.zip");

        export_zip_to(&archive_path, &repository, &store).expect("export");

        let file = fs::File::open(&archive_path).expect("archive");
        let mut archive = zip::ZipArchive::new(file).expect("zip");
        assert_eq!(
            archive.by_index(0).expect("first entry").name(),
            "manifest.json"
        );
        let mut manifest_json = String::new();
        archive
            .by_name("manifest.json")
            .expect("manifest")
            .read_to_string(&mut manifest_json)
            .expect("read manifest");
        let manifest: serde_json::Value = serde_json::from_str(&manifest_json).expect("json");
        assert_eq!(manifest["version"], 2);
        assert_eq!(manifest["item_count"], 1);
        assert!(manifest["exported_at"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert_eq!(manifest["items"][0]["content"], "完整正文\n第二行");
        assert_eq!(
            manifest["items"][0]["content_path"],
            serde_json::Value::Null
        );

        drop(archive);
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn export_rewrites_image_path_and_streams_the_blob_entry() {
        let root = temp_dir("image-path");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let staged = crate::blobs::image::stage_dib(store.stage_dir(), dib32([3, 2, 1, 255]))
            .expect("stage image");
        let installed = store
            .install_staged_with(staged, |image| Ok(image.clone()), |_| Ok(false))
            .expect("install image");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(installed.bmp_path().to_string_lossy().into_owned()),
                content_hash: Some(installed.content_hash().to_string()),
                preview: "image".to_string(),
                source_app: None,
                size_bytes: fs::metadata(installed.bmp_path()).expect("metadata").len() as i64,
            })
            .expect("insert image");
        let archive_path = root.join("backup.zip");

        export_zip_to(&archive_path, &repository, &store).expect("export");

        let file = fs::File::open(&archive_path).expect("archive");
        let mut archive = zip::ZipArchive::new(file).expect("zip");
        let expected_entry = format!("blobs/{}.bmp", installed.content_hash());
        let manifest: serde_json::Value =
            serde_json::from_reader(archive.by_name("manifest.json").expect("manifest"))
                .expect("manifest json");
        assert_eq!(manifest["items"][0]["content_path"], expected_entry);
        assert_eq!(
            manifest["items"][0]["content_hash"],
            installed.content_hash()
        );
        assert!(!manifest
            .to_string()
            .contains(&root.to_string_lossy().to_string()));
        let mut archived_blob = Vec::new();
        archive
            .by_name(&expected_entry)
            .expect("blob entry")
            .read_to_end(&mut archived_blob)
            .expect("read blob");
        assert_eq!(
            archived_blob,
            fs::read(installed.bmp_path()).expect("source blob")
        );

        drop(archive);
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn export_streams_header_valid_blob_without_full_image_decode() {
        let root = temp_dir("opaque-bmp");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let content_hash = "0".repeat(64);
        let source_path = store.blob_dir().join(format!("{content_hash}.bmp"));
        let source_bytes = opaque_bmp(2 * 1024 * 1024);
        fs::write(&source_path, &source_bytes).expect("opaque BMP");
        let item = ClipboardItem {
            id: "opaque".to_string(),
            hash: "record-opaque".to_string(),
            item_type: "image".to_string(),
            content: None,
            content_path: Some(source_path.to_string_lossy().into_owned()),
            content_hash: Some(content_hash.clone()),
            preview: "opaque".to_string(),
            source_app: None,
            favorite: false,
            pinned: false,
            size_bytes: i64::try_from(source_bytes.len()).expect("size"),
            created_at: 1,
            updated_at: 1,
        };
        let archive_path = root.join("backup.zip");

        export_snapshot_to_with_hook(&archive_path, vec![item], &store, || Ok(()))
            .expect("header validation must not decode the full image");

        let file = fs::File::open(&archive_path).expect("archive");
        let mut archive = zip::ZipArchive::new(file).expect("zip");
        let mut archived = Vec::new();
        archive
            .by_name(&format!("blobs/{content_hash}.bmp"))
            .expect("blob")
            .read_to_end(&mut archived)
            .expect("read archived blob");
        assert_eq!(archived, source_bytes);

        drop(archive);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn export_streams_from_the_same_handle_that_was_validated() {
        let root = temp_dir("same-handle");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let content_hash = "2".repeat(64);
        let source_path = store.blob_dir().join(format!("{content_hash}.bmp"));
        let mut original_bytes = opaque_bmp(64);
        original_bytes[14..].fill(0x11);
        fs::write(&source_path, &original_bytes).expect("original BMP");
        let replacement_path = root.join("replacement.bmp");
        let mut replacement_bytes = opaque_bmp(64);
        replacement_bytes[14..].fill(0x22);
        fs::write(&replacement_path, replacement_bytes).expect("replacement BMP");
        let item = ClipboardItem {
            id: "same-handle".to_string(),
            hash: "record-same-handle".to_string(),
            item_type: "image".to_string(),
            content: None,
            content_path: Some(source_path.to_string_lossy().into_owned()),
            content_hash: Some(content_hash.clone()),
            preview: "same-handle".to_string(),
            source_app: None,
            favorite: false,
            pinned: false,
            size_bytes: i64::try_from(original_bytes.len()).expect("size"),
            created_at: 1,
            updated_at: 1,
        };
        let archive_path = root.join("backup.zip");
        let moved_original = root.join("opened-original.bmp");

        export_snapshot_to_with_hook(&archive_path, vec![item], &store, || {
            fs::rename(&source_path, &moved_original)?;
            fs::rename(&replacement_path, &source_path)?;
            Ok(())
        })
        .expect("export");

        let file = fs::File::open(&archive_path).expect("archive");
        let mut archive = zip::ZipArchive::new(file).expect("zip");
        let mut archived = Vec::new();
        archive
            .by_name(&format!("blobs/{content_hash}.bmp"))
            .expect("blob")
            .read_to_end(&mut archived)
            .expect("read archived blob");
        assert_eq!(archived, original_bytes);

        drop(archive);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn export_writes_one_blob_for_duplicate_content_hashes() {
        let root = temp_dir("deduplicate");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let staged = crate::blobs::image::stage_dib(store.stage_dir(), dib32([9, 8, 7, 255]))
            .expect("stage image");
        let installed = store
            .install_staged_with(staged, |image| Ok(image.clone()), |_| Ok(false))
            .expect("install image");
        let items = [("one", "record-one"), ("two", "record-two")]
            .into_iter()
            .map(|(id, record_hash)| ClipboardItem {
                id: id.to_string(),
                hash: record_hash.to_string(),
                item_type: "image".to_string(),
                content: None,
                content_path: Some(installed.bmp_path().to_string_lossy().into_owned()),
                content_hash: Some(installed.content_hash().to_string()),
                preview: id.to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 58,
                created_at: 1,
                updated_at: if id == "one" { 1 } else { 2 },
            })
            .collect();
        let archive_path = root.join("backup.zip");

        export_snapshot_to_with_hook(&archive_path, items, &store, || Ok(())).expect("export");

        let file = fs::File::open(&archive_path).expect("archive");
        let archive = zip::ZipArchive::new(file).expect("zip");
        assert_eq!(archive.len(), 2, "manifest plus one unique blob");
        drop(archive);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn export_rejects_missing_unsafe_and_mismatched_image_identity_before_replacing_target() {
        let root = temp_dir("invalid-images");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let staged = crate::blobs::image::stage_dib(store.stage_dir(), dib32([6, 5, 4, 255]))
            .expect("stage image");
        let installed = store
            .install_staged_with(staged, |image| Ok(image.clone()), |_| Ok(false))
            .expect("install image");
        let target = root.join("backup.zip");
        fs::write(&target, b"existing backup").expect("existing target");
        let valid = ClipboardItem {
            id: "image".to_string(),
            hash: "record-hash".to_string(),
            item_type: "image".to_string(),
            content: None,
            content_path: Some(installed.bmp_path().to_string_lossy().into_owned()),
            content_hash: Some(installed.content_hash().to_string()),
            preview: "image".to_string(),
            source_app: None,
            favorite: false,
            pinned: false,
            size_bytes: 58,
            created_at: 1,
            updated_at: 1,
        };

        let mut missing_hash = valid.clone();
        missing_hash.content_hash = None;
        assert!(
            export_snapshot_to_with_hook(&target, vec![missing_hash], &store, || Ok(())).is_err()
        );
        let mut missing_path = valid.clone();
        missing_path.content_path = None;
        assert!(
            export_snapshot_to_with_hook(&target, vec![missing_path], &store, || Ok(())).is_err()
        );
        let outside = root.join("outside.bmp");
        fs::copy(installed.bmp_path(), &outside).expect("outside blob");
        let mut escaped = valid.clone();
        escaped.content_path = Some(outside.to_string_lossy().into_owned());
        assert!(export_snapshot_to_with_hook(&target, vec![escaped], &store, || Ok(())).is_err());
        let mut uppercase_hash = valid.clone();
        uppercase_hash.content_hash = Some(installed.content_hash().to_uppercase());
        assert!(
            export_snapshot_to_with_hook(&target, vec![uppercase_hash], &store, || Ok(())).is_err()
        );
        let mismatched_hash = "0".repeat(64);
        let mut mismatched = valid.clone();
        mismatched.content_hash = Some(mismatched_hash);
        assert!(
            export_snapshot_to_with_hook(&target, vec![mismatched], &store, || Ok(())).is_err()
        );
        let invalid_header_hash = "1".repeat(64);
        let invalid_header_path = store.blob_dir().join(format!("{invalid_header_hash}.bmp"));
        let mut invalid_header_bytes = fs::read(installed.bmp_path()).expect("source BMP");
        invalid_header_bytes[2..6].copy_from_slice(&1u32.to_le_bytes());
        fs::write(&invalid_header_path, invalid_header_bytes).expect("invalid header blob");
        let mut invalid_header = valid;
        invalid_header.content_hash = Some(invalid_header_hash);
        invalid_header.content_path = Some(invalid_header_path.to_string_lossy().into_owned());
        assert!(
            export_snapshot_to_with_hook(&target, vec![invalid_header], &store, || Ok(())).is_err()
        );
        assert_eq!(fs::read(&target).expect("target"), b"existing backup");

        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    struct CountingReader {
        inner: Cursor<Vec<u8>>,
        max_request: usize,
    }

    impl Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.max_request = self.max_request.max(buffer.len());
            self.inner.read(buffer)
        }
    }

    #[test]
    fn export_blob_copy_bounds_each_read_and_produces_a_readable_zip() {
        let payload = vec![0x5a; 2 * 1024 * 1024];
        let mut reader = CountingReader {
            inner: Cursor::new(payload.clone()),
            max_request: 0,
        };
        let output = Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(output);

        write_blob_entry(&mut archive, "blobs/large.bmp", &mut reader).expect("stream blob");
        let output = archive.finish().expect("finish archive");

        assert!(
            reader.max_request <= 64 * 1024,
            "max read was {}",
            reader.max_request
        );
        let mut archive = zip::ZipArchive::new(output).expect("read zip");
        let mut round_trip = Vec::new();
        archive
            .by_name("blobs/large.bmp")
            .expect("blob")
            .read_to_end(&mut round_trip)
            .expect("read blob");
        assert_eq!(round_trip, payload);
    }

    #[test]
    fn export_releases_repository_snapshot_lock_and_holds_blob_read_lease() {
        let root = temp_dir("leases");
        fs::create_dir_all(&root).expect("root");
        let repository = Arc::new(Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        ));
        let store = Arc::new(
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store"),
        );
        let (hook_entered_tx, hook_entered_rx) = mpsc::channel();
        let (release_hook_tx, release_hook_rx) = mpsc::channel();
        let (writer_entered_tx, writer_entered_rx) = mpsc::channel();
        let worker_repository = Arc::clone(&repository);
        let worker_store = Arc::clone(&store);
        let archive_path = root.join("backup.zip");
        let exporter = thread::spawn(move || {
            export_zip_to_with_hook(&archive_path, &worker_repository, &worker_store, || {
                anyhow::ensure!(
                    worker_repository.try_lock().is_ok(),
                    "repository lock held across archive IO"
                );
                hook_entered_tx.send(()).expect("signal hook");
                release_hook_rx.recv().expect("release hook");
                Ok(())
            })
        });
        hook_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("hook entered");

        let writer_store = Arc::clone(&store);
        let writer = thread::spawn(move || {
            writer_store.with_write(|_, _| {
                writer_entered_tx.send(()).expect("signal writer");
                Ok(())
            })
        });
        assert!(writer_entered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_hook_tx.send(()).expect("release exporter");
        writer_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer entered after export read lease");
        exporter.join().expect("exporter thread").expect("export");
        writer.join().expect("writer thread").expect("writer");

        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn export_failure_preserves_existing_target_and_removes_partial_temp() {
        let root = temp_dir("atomic-failure");
        fs::create_dir_all(&root).expect("root");
        let target = root.join("backup.zip");
        fs::write(&target, b"existing complete backup").expect("existing target");

        let error = write_backup_atomically(&target, |file| {
            file.write_all(b"partial new archive")?;
            Err::<(), anyhow::Error>(anyhow::anyhow!("injected archive failure"))
        })
        .expect_err("write must fail");

        assert!(error.to_string().contains("injected archive failure"));
        assert_eq!(
            fs::read(&target).expect("target"),
            b"existing complete backup"
        );
        let leftovers = fs::read_dir(&root)
            .expect("read root")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_detects_zip_magic_and_keeps_legacy_json_compatible() {
        let root = temp_dir("parse-formats");
        fs::create_dir_all(&root).expect("root");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let zip_with_json_extension = root.join("backup.json");
        export_zip_to(&zip_with_json_extension, &repository, &store).expect("export zip");

        let zip_info = parse_backup_info_path(&zip_with_json_extension).expect("zip info");
        assert_eq!(zip_info.version, "2");
        assert_eq!(zip_info.item_count, 0);
        assert!(!zip_info.created_at.is_empty());

        let legacy_path = root.join("legacy.json");
        let legacy = crate::commands::BackupData {
            metadata: crate::commands::BackupMetadata {
                version: "1.0".to_string(),
                created_at: "2026-07-15T00:00:00Z".to_string(),
                item_count: 0,
            },
            items: Vec::new(),
            blobs: Vec::new(),
        };
        fs::write(
            &legacy_path,
            serde_json::to_vec(&legacy).expect("legacy json"),
        )
        .expect("write legacy");
        let legacy_info = parse_backup_info_path(&legacy_path).expect("legacy info");
        assert_eq!(legacy_info.version, "1.0");
        assert_eq!(legacy_info.item_count, 0);
        assert_eq!(legacy_info.created_at, "2026-07-15T00:00:00Z");

        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_rejects_legacy_file_larger_than_the_quota_derived_bound() {
        let root = temp_dir("legacy-size-bound");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("oversize.json");
        let file = fs::File::create(&path).expect("sparse legacy file");
        file.set_len(
            crate::storage::capacity::MANAGED_BLOB_QUOTA
                .checked_mul(3)
                .expect("test size"),
        )
        .expect("extend sparse legacy file");

        let error = parse_backup_info_path(&path).expect_err("oversize legacy backup must fail");
        assert!(format!("{error:#}").contains("legacy backup exceeds"));

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_rejects_unsupported_legacy_version_and_count_mismatch() {
        let root = temp_dir("legacy-metadata-validation");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("legacy.json");

        write_legacy_json(&path, "2.0", 0, Vec::new(), Vec::new());
        assert!(
            parse_backup_info_path(&path).is_err(),
            "unsupported legacy version must fail"
        );

        write_legacy_json(&path, "1.0", 2, vec![legacy_item("text", None)], Vec::new());
        assert!(
            parse_backup_info_path(&path).is_err(),
            "legacy item_count mismatch must fail"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_rejects_unsafe_or_overlong_legacy_image_paths() {
        let root = temp_dir("legacy-item-paths");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("legacy.json");

        for unsafe_path in ["../escape.bmp", "C:\\temp\\escape.bmp"] {
            write_legacy_json(
                &path,
                "1.0",
                1,
                vec![legacy_item("image", Some(unsafe_path))],
                Vec::new(),
            );
            assert!(
                parse_backup_info_path(&path).is_err(),
                "unsafe legacy image path must fail: {unsafe_path}"
            );
        }

        let overlong_path = format!("{}.bmp", "a".repeat(1021));
        write_legacy_json(
            &path,
            "1.0",
            1,
            vec![legacy_item("image", Some(&overlong_path))],
            Vec::new(),
        );
        assert!(
            parse_backup_info_path(&path).is_err(),
            "overlong legacy image path must fail"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_rejects_unsafe_aliased_or_non_bmp_legacy_blobs() {
        let root = temp_dir("legacy-blob-names");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("legacy.json");

        for invalid_name in [
            "../escape.bmp",
            "nested/image.bmp",
            "image.png",
            "image.bmp:stream.bmp",
            "CON.bmp",
        ] {
            write_legacy_json(
                &path,
                "1.0",
                0,
                Vec::new(),
                vec![legacy_blob(invalid_name, "AA==")],
            );
            assert!(
                parse_backup_info_path(&path).is_err(),
                "invalid legacy blob filename must fail: {invalid_name}"
            );
        }

        write_legacy_json(
            &path,
            "1.0",
            0,
            Vec::new(),
            vec![
                legacy_blob("image.bmp", "AA=="),
                legacy_blob("IMAGE.BMP", "AQ=="),
            ],
        );
        assert!(
            parse_backup_info_path(&path).is_err(),
            "case-insensitive legacy blob aliases must fail"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_streams_large_legacy_payloads_in_any_field_order() {
        let root = temp_dir("legacy-streaming");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("legacy.json");
        let large_base64 = "A".repeat(2 * 1024 * 1024);
        let json = format!(
            r#"{{"blobs":[{{"data_base64":"{large_base64}","filename":"image.bmp","item_id":"legacy-item"}}],"items":[{{"updated_at":1,"size_bytes":4,"source_app":null,"preview":"legacy","pinned":false,"item_type":"image","id":"legacy-item","hash":"legacy-hash","favorite":false,"created_at":1,"content_path":"image.bmp","content":null}}],"metadata":{{"item_count":1,"created_at":"2026-07-15T00:00:00Z","version":"1.0"}}}}"#
        );
        fs::write(&path, json).expect("write large legacy JSON");

        let info = parse_backup_info_path(&path).expect("valid legacy info");
        assert_eq!(info.version, "1.0");
        assert_eq!(info.item_count, 1);

        fs::write(&path, br#"{"metadata":{"version":"1.0","created_at":"test","item_count":0},"items":[],"blobs":[]} trailing"#)
            .expect("write trailing JSON");
        assert!(
            parse_backup_info_path(&path).is_err(),
            "trailing non-whitespace must fail"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_rejects_malformed_count_and_traversal_manifests() {
        let root = temp_dir("parse-invalid");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("backup.bin");
        fs::write(&path, b"PK\x03\x04not a real zip").expect("malformed ZIP");
        assert!(
            parse_backup_info_path(&path).is_err(),
            "ZIP magic must not fall back to JSON"
        );

        write_test_zip(
            &path,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-15T00:00:00Z",
                "item_count": 1,
                "items": []
            }),
            &[],
        );
        assert!(
            parse_backup_info_path(&path).is_err(),
            "count mismatch must fail"
        );

        let hash = "0".repeat(64);
        write_test_zip(
            &path,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-15T00:00:00Z",
                "item_count": 1,
                "items": [manifest_image("../escape.bmp", &hash)]
            }),
            &[&format!("blobs/{hash}.bmp")],
        );
        assert!(
            parse_backup_info_path(&path).is_err(),
            "traversal path must fail"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_rejects_manifest_declared_over_the_bound() {
        let root = temp_dir("parse-oversize");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("backup.zip");
        write_test_zip(&path, &serde_json::json!({}), &[]);
        let mut bytes = fs::read(&path).expect("read zip bytes");
        let central = bytes
            .windows(4)
            .position(|window| window == b"PK\x01\x02")
            .expect("central directory");
        let declared_size = u32::try_from(MAX_MANIFEST_BYTES + 1).expect("test bound fits u32");
        bytes[central + 24..central + 28].copy_from_slice(&declared_size.to_le_bytes());
        fs::write(&path, bytes).expect("patch declared size");

        let error = parse_backup_info_path(&path).expect_err("oversize manifest must fail");
        assert!(error.to_string().contains("manifest exceeds"));

        fs::remove_dir_all(root).expect("cleanup");
    }
}
