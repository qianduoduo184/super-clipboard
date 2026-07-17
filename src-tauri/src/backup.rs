use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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
    let is_bmp = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bmp"));
    if !is_bmp {
        return Err("legacy blob filename must use the .bmp extension".to_string());
    }
    let device_base = filename
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    let is_windows_device = matches!(
        device_base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$"
    ) || device_base
        .strip_prefix("COM")
        .or_else(|| device_base.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        });
    if is_windows_device {
        return Err("legacy blob filename must not use a Windows device alias".to_string());
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

const MAX_ZIP_ENTRIES: u64 = 100_001;
const MAX_CENTRAL_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ZIP_ENTRY_NAME_BYTES: usize = 1024;
const ZIP32_EOCD_LEN: u64 = 22;
const ZIP64_LOCATOR_LEN: u64 = 20;
const CENTRAL_HEADER_LEN: u64 = 46;

struct RawCentralDirectory {
    entry_count: u64,
    offset: u64,
    size: u64,
    metadata_offset: u64,
}

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("fixed ZIP u16"))
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed ZIP u32"))
}

fn le_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("fixed ZIP u64"))
}

fn read_zip_bytes<const LEN: usize>(file: &mut File, offset: u64) -> anyhow::Result<[u8; LEN]> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = [0u8; LEN];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn find_standard_eocd(file: &mut File, file_len: u64) -> anyhow::Result<(u64, [u8; 22])> {
    anyhow::ensure!(
        file_len >= ZIP32_EOCD_LEN,
        "ZIP file is shorter than the EOCD"
    );
    let search_len = file_len.min(ZIP32_EOCD_LEN + u64::from(u16::MAX));
    let search_start = file_len - search_len;
    file.seek(SeekFrom::Start(search_start))?;
    let mut tail = vec![0u8; usize::try_from(search_len).expect("bounded EOCD search")];
    file.read_exact(&mut tail)?;
    for index in (0..=tail.len() - ZIP32_EOCD_LEN as usize).rev() {
        if &tail[index..index + 4] != b"PK\x05\x06" {
            continue;
        }
        let comment_len = usize::from(le_u16(&tail, index + 20));
        if index
            .checked_add(ZIP32_EOCD_LEN as usize)
            .and_then(|end| end.checked_add(comment_len))
            == Some(tail.len())
        {
            let mut eocd = [0u8; 22];
            eocd.copy_from_slice(&tail[index..index + 22]);
            return Ok((search_start + index as u64, eocd));
        }
    }
    Err(anyhow::anyhow!(
        "ZIP EOCD was not found at the end of the file"
    ))
}

fn read_raw_central_directory(
    file: &mut File,
    file_len: u64,
) -> anyhow::Result<RawCentralDirectory> {
    let (eocd_offset, eocd) = find_standard_eocd(file, file_len)?;
    anyhow::ensure!(
        le_u16(&eocd, 4) == 0 && le_u16(&eocd, 6) == 0,
        "multi-disk ZIP archives are not supported"
    );
    let entries_on_disk = le_u16(&eocd, 8);
    let entries = le_u16(&eocd, 10);
    let size32 = le_u32(&eocd, 12);
    let offset32 = le_u32(&eocd, 16);

    let locator = eocd_offset
        .checked_sub(ZIP64_LOCATOR_LEN)
        .and_then(|offset| {
            read_zip_bytes::<20>(file, offset)
                .ok()
                .map(|bytes| (offset, bytes))
        })
        .filter(|(_, bytes)| &bytes[..4] == b"PK\x06\x07");
    let requires_zip64 = size32 == u32::MAX || offset32 == u32::MAX;
    if requires_zip64 {
        anyhow::ensure!(locator.is_some(), "ZIP64 locator is missing");
    }

    if let Some((locator_offset, locator)) = locator {
        anyhow::ensure!(
            le_u32(&locator, 4) == 0 && le_u32(&locator, 16) == 1,
            "multi-disk ZIP64 archives are not supported"
        );
        let zip64_offset = le_u64(&locator, 8);
        anyhow::ensure!(
            zip64_offset < locator_offset,
            "ZIP64 EOCD offset is outside the ZIP file"
        );
        let zip64 =
            read_zip_bytes::<56>(file, zip64_offset).context("read ZIP64 EOCD fixed fields")?;
        anyhow::ensure!(&zip64[..4] == b"PK\x06\x06", "invalid ZIP64 EOCD signature");
        let record_size = le_u64(&zip64, 4);
        anyhow::ensure!(record_size >= 44, "ZIP64 EOCD record is too short");
        let record_end = zip64_offset
            .checked_add(12)
            .and_then(|offset| offset.checked_add(record_size))
            .ok_or_else(|| anyhow::anyhow!("ZIP64 EOCD bounds overflow"))?;
        anyhow::ensure!(
            record_end == locator_offset,
            "ZIP64 EOCD must end at its locator"
        );
        anyhow::ensure!(
            le_u32(&zip64, 16) == 0 && le_u32(&zip64, 20) == 0,
            "multi-disk ZIP64 archives are not supported"
        );
        let entries_on_disk = le_u64(&zip64, 24);
        let entries = le_u64(&zip64, 32);
        anyhow::ensure!(
            entries_on_disk == entries,
            "ZIP64 entry counts differ across disks"
        );
        return Ok(RawCentralDirectory {
            entry_count: entries,
            size: le_u64(&zip64, 40),
            offset: le_u64(&zip64, 48),
            metadata_offset: zip64_offset,
        });
    }

    anyhow::ensure!(
        entries_on_disk == entries,
        "ZIP entry counts differ across disks"
    );
    Ok(RawCentralDirectory {
        entry_count: u64::from(entries),
        size: u64::from(size32),
        offset: u64::from(offset32),
        metadata_offset: eocd_offset,
    })
}

fn normalize_raw_zip_name(name: &[u8]) -> anyhow::Result<String> {
    anyhow::ensure!(!name.is_empty(), "ZIP entry name is empty");
    anyhow::ensure!(
        name.len() <= MAX_ZIP_ENTRY_NAME_BYTES,
        "ZIP entry name exceeds the bounded backup path limit"
    );
    anyhow::ensure!(name.is_ascii(), "ZIP entry name must be ASCII");
    anyhow::ensure!(
        !matches!(name.first(), Some(b'/') | Some(b'\\'))
            && !matches!(name.last(), Some(b'/') | Some(b'\\')),
        "ZIP entry name must be a relative file path"
    );
    let name = std::str::from_utf8(name).expect("ASCII ZIP name");
    let mut normalized = Vec::new();
    for component in name.split(['/', '\\']) {
        anyhow::ensure!(
            !component.is_empty() && component != "." && component != "..",
            "ZIP entry name contains an unsafe path component"
        );
        normalized.push(component);
    }
    Ok(normalized.join("/"))
}

fn zip64_local_header_offset(fixed: &[u8; 46], extra: &[u8]) -> anyhow::Result<u64> {
    let raw_offset = le_u32(fixed, 42);
    if raw_offset != u32::MAX {
        return Ok(u64::from(raw_offset));
    }
    let need_uncompressed = le_u32(fixed, 24) == u32::MAX;
    let need_compressed = le_u32(fixed, 20) == u32::MAX;
    let mut cursor = 0usize;
    while cursor < extra.len() {
        let header_end = cursor
            .checked_add(4)
            .ok_or_else(|| anyhow::anyhow!("ZIP extra field bounds overflow"))?;
        anyhow::ensure!(
            header_end <= extra.len(),
            "truncated ZIP extra field header"
        );
        let tag = le_u16(extra, cursor);
        let len = usize::from(le_u16(extra, cursor + 2));
        let value_end = header_end
            .checked_add(len)
            .ok_or_else(|| anyhow::anyhow!("ZIP extra field bounds overflow"))?;
        anyhow::ensure!(value_end <= extra.len(), "truncated ZIP extra field value");
        if tag == 0x0001 {
            let mut value_cursor = header_end;
            if need_uncompressed {
                value_cursor = value_cursor
                    .checked_add(8)
                    .ok_or_else(|| anyhow::anyhow!("ZIP64 extra field bounds overflow"))?;
            }
            if need_compressed {
                value_cursor = value_cursor
                    .checked_add(8)
                    .ok_or_else(|| anyhow::anyhow!("ZIP64 extra field bounds overflow"))?;
            }
            let offset_end = value_cursor
                .checked_add(8)
                .ok_or_else(|| anyhow::anyhow!("ZIP64 extra field bounds overflow"))?;
            anyhow::ensure!(offset_end <= value_end, "ZIP64 local offset is missing");
            return Ok(le_u64(extra, value_cursor));
        }
        cursor = value_end;
    }
    Err(anyhow::anyhow!("ZIP64 local offset extra field is missing"))
}

fn preflight_raw_zip(file: &mut File) -> anyhow::Result<()> {
    let file_len = file.metadata()?.len();
    let directory = read_raw_central_directory(file, file_len)?;
    anyhow::ensure!(
        directory.entry_count <= MAX_ZIP_ENTRIES,
        "ZIP entry count exceeds the backup limit"
    );
    anyhow::ensure!(
        directory.size <= MAX_CENTRAL_DIRECTORY_BYTES,
        "ZIP central directory exceeds the backup limit"
    );
    let minimum_size = directory
        .entry_count
        .checked_mul(CENTRAL_HEADER_LEN)
        .ok_or_else(|| anyhow::anyhow!("ZIP central directory entry count overflow"))?;
    anyhow::ensure!(
        minimum_size <= directory.size,
        "ZIP entry count exceeds declared central directory size"
    );
    let central_end = directory
        .offset
        .checked_add(directory.size)
        .ok_or_else(|| anyhow::anyhow!("ZIP central directory bounds overflow"))?;
    anyhow::ensure!(
        central_end == directory.metadata_offset && central_end <= file_len,
        "ZIP central directory is outside the ZIP file"
    );

    let mut cursor = directory.offset;
    let mut names = HashSet::new();
    let mut previous_local_offset = None;
    for index in 0..directory.entry_count {
        let fixed_end = cursor
            .checked_add(CENTRAL_HEADER_LEN)
            .ok_or_else(|| anyhow::anyhow!("ZIP central header bounds overflow"))?;
        anyhow::ensure!(fixed_end <= central_end, "truncated ZIP central header");
        let fixed = read_zip_bytes::<46>(file, cursor)?;
        anyhow::ensure!(
            &fixed[..4] == b"PK\x01\x02",
            "invalid ZIP central header signature"
        );
        let name_len = usize::from(le_u16(&fixed, 28));
        let extra_len = usize::from(le_u16(&fixed, 30));
        let comment_len = usize::from(le_u16(&fixed, 32));
        anyhow::ensure!(
            le_u16(&fixed, 34) == 0,
            "multi-disk ZIP entries are not supported"
        );
        let variable_len = name_len
            .checked_add(extra_len)
            .and_then(|length| length.checked_add(comment_len))
            .ok_or_else(|| anyhow::anyhow!("ZIP central entry length overflow"))?;
        let entry_end = fixed_end
            .checked_add(variable_len as u64)
            .ok_or_else(|| anyhow::anyhow!("ZIP central entry bounds overflow"))?;
        anyhow::ensure!(
            entry_end <= central_end,
            "ZIP central entry exceeds declared bounds"
        );

        file.seek(SeekFrom::Start(fixed_end))?;
        let mut raw_name = vec![0u8; name_len];
        file.read_exact(&mut raw_name)?;
        let name = normalize_raw_zip_name(&raw_name)?;
        anyhow::ensure!(
            names.insert(name.clone()),
            "duplicate normalized ZIP entry: {name}"
        );
        if index == 0 {
            anyhow::ensure!(
                name == "manifest.json",
                "manifest.json must be the first ZIP entry"
            );
        }

        let mut extra = vec![0u8; extra_len];
        file.read_exact(&mut extra)?;
        let local_offset = zip64_local_header_offset(&fixed, &extra)?;
        anyhow::ensure!(
            local_offset < directory.offset,
            "ZIP local header is outside the file data region"
        );
        if let Some(previous) = previous_local_offset {
            anyhow::ensure!(
                local_offset > previous,
                "ZIP local header order does not match the central directory"
            );
        } else {
            anyhow::ensure!(local_offset == 0, "manifest local header must be first");
        }
        let local = read_zip_bytes::<30>(file, local_offset)?;
        anyhow::ensure!(
            &local[..4] == b"PK\x03\x04",
            "invalid ZIP local header signature"
        );
        let local_name_len = usize::from(le_u16(&local, 26));
        let local_extra_len = usize::from(le_u16(&local, 28));
        let local_end = local_offset
            .checked_add(30)
            .and_then(|offset| offset.checked_add(local_name_len as u64))
            .and_then(|offset| offset.checked_add(local_extra_len as u64))
            .ok_or_else(|| anyhow::anyhow!("ZIP local header bounds overflow"))?;
        anyhow::ensure!(
            local_end <= directory.offset,
            "ZIP local header exceeds file data bounds"
        );
        previous_local_offset = Some(local_offset);
        cursor = entry_end;
    }
    anyhow::ensure!(
        cursor == central_end,
        "ZIP central directory size does not match entries"
    );
    Ok(())
}

fn parse_zip_info(mut file: File) -> anyhow::Result<crate::commands::BackupInfo> {
    preflight_raw_zip(&mut file).context("preflight raw ZIP metadata")?;
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ImportMode {
    Merge,
    Overwrite,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub safety_backup: Option<std::path::PathBuf>,
}

pub fn import_backup_from_path(
    path: &Path,
    mode: ImportMode,
    repository: &Mutex<ClipboardRepository>,
    blob_store: &ImageBlobStore,
    safety_backup_dir: &Path,
) -> anyhow::Result<ImportResult> {
    let mut file = File::open(path).with_context(|| format!("open backup {}", path.display()))?;
    let mut magic = [0u8; 4];
    let count = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    if count == magic.len() && is_zip_magic(magic) {
        return import_zip_backup_from_path(path, mode, repository, blob_store, safety_backup_dir);
    }
    import_legacy_backup_as_zip(file, path, mode, repository, blob_store, safety_backup_dir)
}

fn import_legacy_backup_as_zip(
    file: File,
    path: &Path,
    mode: ImportMode,
    repository: &Mutex<ClipboardRepository>,
    blob_store: &ImageBlobStore,
    safety_backup_dir: &Path,
) -> anyhow::Result<ImportResult> {
    let backup: crate::commands::BackupData = serde_json::from_reader(file)
        .with_context(|| format!("parse legacy backup {}", path.display()))?;
    anyhow::ensure!(
        backup.metadata.version == "1.0",
        "unsupported legacy backup version"
    );
    anyhow::ensure!(
        backup.metadata.item_count == backup.items.len(),
        "legacy backup item_count does not match items"
    );

    let temp_zip = blob_store
        .stage_dir()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".legacy-import-{}.zip", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut normalized_items = backup.items;
        let mut staged_by_hash = HashMap::new();
        let normalize_result = (|| {
            for item in &mut normalized_items {
                if item.item_type != "image" {
                    continue;
                }
                let filename = item.content_path.clone().ok_or_else(|| {
                    anyhow::anyhow!("legacy image item {} is missing content_path", item.id)
                })?;
                validate_legacy_bmp_filename(&filename).map_err(|error| anyhow::anyhow!(error))?;
                let blob = backup
                    .blobs
                    .iter()
                    .find(|blob| blob.item_id == item.id && blob.filename == filename)
                    .ok_or_else(|| {
                        anyhow::anyhow!("legacy image item {} is missing blob", item.id)
                    })?;
                let bmp = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    &blob.data_base64,
                )
                .with_context(|| format!("decode legacy image blob {filename}"))?;
                anyhow::ensure!(
                    bmp.len() >= 14 && &bmp[..2] == b"BM",
                    "legacy blob {filename} is not a BMP"
                );
                let staged =
                    crate::blobs::image::stage_dib(blob_store.stage_dir(), bmp[14..].to_vec())
                        .with_context(|| format!("stage legacy image {filename}"))?;
                let content_hash = staged.content_hash().to_string();
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    staged_by_hash.entry(content_hash.clone())
                {
                    entry.insert(staged);
                } else {
                    fs::remove_dir_all(staged.stage_dir()).with_context(|| {
                        format!(
                            "remove duplicate normalized legacy image stage {}",
                            staged.stage_dir().display()
                        )
                    })?;
                }
                item.hash = format!("image:{content_hash}");
                item.content_hash = Some(content_hash.clone());
                item.content_path = Some(format!("blobs/{content_hash}.bmp"));
            }
            Ok::<(), anyhow::Error>(())
        })();
        if let Err(error) = normalize_result {
            return Err(combine_with_stage_cleanup(error, &staged_by_hash));
        }

        let write_result = (|| {
            let file = File::create(&temp_zip).with_context(|| {
                format!("create normalized legacy backup {}", temp_zip.display())
            })?;
            let mut archive = ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            archive.start_file("manifest.json", options)?;
            let manifest = BackupManifest {
                version: 2,
                exported_at: backup.metadata.created_at,
                item_count: normalized_items.len(),
                items: normalized_items,
            };
            serde_json::to_writer(&mut archive, &manifest)?;
            let mut content_hashes = staged_by_hash.keys().cloned().collect::<Vec<_>>();
            content_hashes.sort_unstable();
            for content_hash in content_hashes {
                let staged = staged_by_hash
                    .get(&content_hash)
                    .expect("selected normalized legacy image exists");
                archive.start_file(format!("blobs/{content_hash}.bmp"), options)?;
                let mut normalized_bmp = File::open(staged.bmp_path())
                    .with_context(|| format!("open normalized legacy image {content_hash}"))?;
                std::io::copy(&mut normalized_bmp, &mut archive)
                    .with_context(|| format!("stream normalized legacy image {content_hash}"))?;
            }
            archive.finish()?;
            Ok::<(), anyhow::Error>(())
        })();
        if let Err(error) = write_result {
            return Err(combine_with_stage_cleanup(error, &staged_by_hash));
        }
        cleanup_import_stages(&staged_by_hash)?;
        import_zip_backup_from_path(&temp_zip, mode, repository, blob_store, safety_backup_dir)
    })();
    finalize_temp_import_cleanup(result, fs::remove_file(&temp_zip), &temp_zip)
}

fn cleanup_import_stages(
    staged_by_hash: &HashMap<String, crate::blobs::image::StagedImage>,
) -> anyhow::Result<()> {
    let mut first_error = None;
    for staged in staged_by_hash.values() {
        if let Err(error) = fs::remove_dir_all(staged.stage_dir()) {
            if error.kind() != std::io::ErrorKind::NotFound && first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error).context("remove normalized legacy image stages"),
        None => Ok(()),
    }
}

fn combine_with_stage_cleanup(
    error: anyhow::Error,
    staged_by_hash: &HashMap<String, crate::blobs::image::StagedImage>,
) -> anyhow::Error {
    match cleanup_import_stages(staged_by_hash) {
        Ok(()) => error,
        Err(cleanup_error) => anyhow::anyhow!(
            "{error:#}; additionally failed to remove normalized legacy image stages: {cleanup_error:#}"
        ),
    }
}

fn finalize_temp_import_cleanup(
    result: anyhow::Result<ImportResult>,
    cleanup_result: std::io::Result<()>,
    temp_zip: &Path,
) -> anyhow::Result<ImportResult> {
    match result {
        Ok(value) => {
            if let Err(error) = cleanup_result {
                if error.kind() != std::io::ErrorKind::NotFound {
                    crate::diagnostics::warn(format!(
                        "backup: committed legacy import but failed to remove normalized backup {}: {}",
                        temp_zip.display(),
                        error
                    ));
                }
            }
            Ok(value)
        }
        Err(error) => match cleanup_result {
            Ok(()) => Err(error),
            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => Err(error),
            Err(cleanup_error) => Err(anyhow::anyhow!(
                "{error:#}; additionally failed to remove normalized legacy backup {}: {cleanup_error}",
                temp_zip.display()
            )),
        },
    }
}

fn import_zip_backup_from_path(
    path: &Path,
    mode: ImportMode,
    repository: &Mutex<ClipboardRepository>,
    blob_store: &ImageBlobStore,
    safety_backup_dir: &Path,
) -> anyhow::Result<ImportResult> {
    let mut file = File::open(path).with_context(|| format!("open backup {}", path.display()))?;
    let mut magic = [0u8; 4];
    let count = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    anyhow::ensure!(
        count == magic.len() && is_zip_magic(magic),
        "legacy JSON import is not implemented"
    );
    preflight_raw_zip(&mut file).context("preflight raw ZIP metadata")?;
    let mut archive = zip::ZipArchive::new(file).context("parse ZIP backup")?;
    anyhow::ensure!(!archive.is_empty(), "ZIP backup is empty");
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
        serde_json::from_reader(manifest_file.take(MAX_MANIFEST_BYTES + 1))
            .context("parse backup manifest")?
    };
    anyhow::ensure!(manifest.version == 2, "unsupported backup manifest version");
    anyhow::ensure!(
        manifest.item_count == manifest.items.len(),
        "backup manifest item_count does not match items"
    );
    let mut expected_blob_paths = HashSet::new();
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
        expected_blob_paths.insert(expected_path);
    }
    let actual_blob_paths = entry_names
        .into_iter()
        .filter(|name| name != "manifest.json")
        .collect::<HashSet<_>>();
    anyhow::ensure!(
        actual_blob_paths == expected_blob_paths,
        "backup ZIP blob entries do not match manifest image references"
    );

    let mut staged_by_hash = HashMap::new();
    let stage_result = (|| {
        for archive_path in &expected_blob_paths {
            let content_hash =
                archive_blob_hash(archive_path).expect("validated archive blob path");
            let entry = archive.by_name(archive_path)?;
            let declared_size = entry.size();
            anyhow::ensure!(
                declared_size <= crate::storage::capacity::MANAGED_BLOB_QUOTA,
                "backup image exceeds managed blob quota"
            );
            let capacity = usize::try_from(declared_size).context("backup image is too large")?;
            let mut bmp = Vec::new();
            bmp.try_reserve_exact(capacity)
                .context("reserve backup image buffer")?;
            entry
                .take(crate::storage::capacity::MANAGED_BLOB_QUOTA + 1)
                .read_to_end(&mut bmp)?;
            anyhow::ensure!(
                bmp.len() as u64 == declared_size,
                "backup image size does not match ZIP metadata"
            );
            anyhow::ensure!(
                bmp.len() >= 14 && &bmp[..2] == b"BM",
                "backup image is not a BMP"
            );
            let dib = bmp.split_off(14);
            let staged = crate::blobs::image::stage_dib(blob_store.stage_dir(), dib)?;
            if staged.content_hash() != content_hash {
                let stage_dir = staged.stage_dir().to_path_buf();
                fs::remove_dir_all(&stage_dir).with_context(|| {
                    format!("remove mismatched image stage {}", stage_dir.display())
                })?;
                anyhow::bail!("backup image semantic hash mismatch for {archive_path}");
            }
            staged_by_hash.insert(content_hash.to_string(), staged);
        }
        Ok::<(), anyhow::Error>(())
    })();
    if let Err(stage_error) = stage_result {
        let mut cleanup_error = None;
        for staged in staged_by_hash.values() {
            if let Err(error) = fs::remove_dir_all(staged.stage_dir()) {
                if cleanup_error.is_none() {
                    cleanup_error = Some(error);
                }
            }
        }
        return match cleanup_error {
            Some(error) => Err(anyhow::anyhow!(
                "{stage_error:#}; additionally failed to clean image stages: {error}"
            )),
            None => Err(stage_error),
        };
    }

    let mut normalized_items = manifest.items;
    for item in &mut normalized_items {
        if item.item_type != "image" {
            continue;
        }
        let content_hash = item.content_hash.as_deref().expect("validated image hash");
        item.content_path = Some(
            crate::blobs::image::canonical_bmp_path(blob_store.blob_dir(), content_hash)?
                .to_string_lossy()
                .into_owned(),
        );
    }

    if mode == ImportMode::Merge {
        let active_hashes = repository
            .lock()
            .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
            .active_hashes()?;
        let required_content_hashes = normalized_items
            .iter()
            .filter(|item| item.item_type == "image" && !active_hashes.contains(&item.hash))
            .filter_map(|item| item.content_hash.clone())
            .collect::<HashSet<_>>();
        let skipped_content_hashes = staged_by_hash
            .keys()
            .filter(|content_hash| !required_content_hashes.contains(*content_hash))
            .cloned()
            .collect::<Vec<_>>();
        for content_hash in skipped_content_hashes {
            let staged = staged_by_hash
                .remove(&content_hash)
                .expect("selected staged image exists");
            fs::remove_dir_all(staged.stage_dir()).with_context(|| {
                format!(
                    "remove duplicate import image stage {}",
                    staged.stage_dir().display()
                )
            })?;
        }
    }

    let repository_mode = match mode {
        ImportMode::Merge => crate::storage::repository::RepositoryImportMode::Merge,
        ImportMode::Overwrite => crate::storage::repository::RepositoryImportMode::Overwrite,
    };
    let safety_backup = if mode == ImportMode::Overwrite {
        match create_import_safety_backup(safety_backup_dir, repository, blob_store) {
            Ok(path) => Some(path),
            Err(error) => {
                for staged in staged_by_hash.values() {
                    let _ = fs::remove_dir_all(staged.stage_dir());
                }
                return Err(error);
            }
        }
    } else {
        None
    };
    blob_store.with_write(|blob_dir, stage_root| {
        crate::commands::cleanup_pending_blobs(repository, blob_dir)?;
        let usage = crate::storage::capacity::managed_usage(blob_dir)?;
        let mut additional = 0u64;
        for staged in staged_by_hash.values() {
            let canonical_bmp =
                crate::blobs::image::canonical_bmp_path(blob_dir, staged.content_hash())?;
            if !canonical_bmp.exists() {
                additional = additional
                    .checked_add(staged.bmp_size())
                    .ok_or_else(|| anyhow::anyhow!("import BMP projection overflow"))?;
            }
            let canonical_thumbnail =
                crate::blobs::image::canonical_thumbnail_path(blob_dir, staged.content_hash())?;
            if !canonical_thumbnail.exists() {
                additional = additional
                    .checked_add(staged.thumbnail_size())
                    .ok_or_else(|| anyhow::anyhow!("import thumbnail projection overflow"))?;
            }
        }
        let reclaimable = if mode == ImportMode::Overwrite {
            let final_paths = normalized_items
                .iter()
                .filter(|item| item.item_type == "image")
                .filter_map(|item| item.content_path.as_deref())
                .map(PathBuf::from)
                .collect::<HashSet<_>>();
            let old_paths = repository
                .lock()
                .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
                .active_blob_paths()?;
            let mut reclaimable = 0u64;
            for old_path in old_paths {
                if !final_paths.contains(&old_path) {
                    reclaimable = reclaimable
                        .checked_add(import_reclaimable_image_bytes(blob_dir, &old_path)?)
                        .ok_or_else(|| anyhow::anyhow!("import reclaimable projection overflow"))?;
                }
            }
            reclaimable
        } else {
            0
        };
        let projected = usage
            .checked_sub(reclaimable)
            .ok_or_else(|| anyhow::anyhow!("import reclaimable usage underflow"))?
            .checked_add(additional)
            .ok_or_else(|| anyhow::anyhow!("import managed usage projection overflow"))?;
        if projected > crate::storage::capacity::MANAGED_BLOB_QUOTA {
            for staged in staged_by_hash.values() {
                fs::remove_dir_all(staged.stage_dir()).with_context(|| {
                    format!(
                        "remove over-quota image stage {}",
                        staged.stage_dir().display()
                    )
                })?;
            }
            anyhow::bail!(
                "managed blob capacity exceeded: projected={projected}, quota={}",
                crate::storage::capacity::MANAGED_BLOB_QUOTA
            );
        }
        let mut installed = Vec::new();
        for staged in staged_by_hash.into_values() {
            installed.push(crate::blobs::store::install_staged_locked(
                blob_dir, stage_root, staged,
            )?);
        }
        let transaction_result = repository
            .lock()
            .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
            .import_items_transactionally(&normalized_items, repository_mode);
        let result = match transaction_result {
            Ok(result) => result,
            Err(error) => {
                clean_unreferenced_import_installs(repository, &installed)?;
                return Err(error);
            }
        };
        clean_unreferenced_import_installs(repository, &installed)?;
        crate::commands::cleanup_pending_blobs(repository, blob_dir)?;
        Ok(ImportResult {
            imported: result.imported,
            skipped: result.skipped,
            safety_backup,
        })
    })
}

fn import_reclaimable_image_bytes(blob_dir: &Path, bmp_path: &Path) -> anyhow::Result<u64> {
    let mut bytes = 0u64;
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
                    .ok_or_else(|| anyhow::anyhow!("import reclaimable byte count overflow"))?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(bytes)
}

fn create_import_safety_backup(
    safety_backup_dir: &Path,
    repository: &Mutex<ClipboardRepository>,
    blob_store: &ImageBlobStore,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(safety_backup_dir).with_context(|| {
        format!(
            "create import safety backup directory {}",
            safety_backup_dir.display()
        )
    })?;
    let filename = format!(
        "super-clipboard-safety-{}-{}.zip",
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%6f"),
        uuid::Uuid::new_v4()
    );
    let path = safety_backup_dir.join(filename);
    export_zip_to(&path, repository, blob_store)
        .with_context(|| format!("create import safety backup {}", path.display()))?;
    Ok(path)
}

fn clean_unreferenced_import_installs(
    repository: &Mutex<ClipboardRepository>,
    installed: &[crate::blobs::store::InstalledImage],
) -> anyhow::Result<()> {
    let repository = repository
        .lock()
        .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
    for image in installed.iter().rev() {
        if repository.any_active_blob_path(&[image.bmp_path().to_path_buf()])? {
            continue;
        }
        for path in image.created_paths().iter().rev() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("rollback imported blob {}", path.display()));
                }
            }
        }
    }
    Ok(())
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
    write_backup_atomically_with_installer(target, write, replace_target)
}

fn write_backup_atomically_with_installer(
    target: &Path,
    write: impl FnOnce(&mut File) -> anyhow::Result<()>,
    install: impl FnOnce(&Path, &Path) -> anyhow::Result<()>,
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
    install(&temp_path, &target)?;
    temp.keep = true;
    Ok(())
}

#[cfg(target_os = "windows")]
fn replace_target(temp_path: &Path, target: &Path) -> anyhow::Result<()> {
    replace_target_with(temp_path, target, |temp_path, target, flags| {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

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
        let moved = unsafe { MoveFileExW(temp_wide.as_ptr(), target_wide.as_ptr(), flags) };
        if moved == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })
}

#[cfg(target_os = "windows")]
fn replace_target_with(
    temp_path: &Path,
    target: &Path,
    move_file: impl FnOnce(&Path, &Path, u32) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    move_file(
        temp_path,
        target,
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
    )
    .with_context(|| {
        format!(
            "install backup {} as {}",
            temp_path.display(),
            target.display()
        )
    })
}

#[cfg(not(target_os = "windows"))]
fn replace_target(temp_path: &Path, target: &Path) -> anyhow::Result<()> {
    fs::rename(temp_path, target).with_context(|| {
        format!(
            "install backup {} as {}",
            temp_path.display(),
            target.display()
        )
    })?;
    #[cfg(unix)]
    File::open(
        target
            .parent()
            .ok_or_else(|| anyhow::anyhow!("backup target parent is missing"))?,
    )
    .context("open backup parent directory for synchronization")?
    .sync_all()
    .context("synchronize backup parent directory")?;
    Ok(())
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
    items: Vec<ClipboardItem>,
    blob_store: &ImageBlobStore,
    hook: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    export_snapshot_to_with_io(
        path,
        items,
        blob_store,
        hook,
        |path| File::open(path),
        |_| Ok(()),
    )
}

trait BlobReadHandle: Read + Seek {
    fn metadata(&self) -> std::io::Result<fs::Metadata>;
}

impl BlobReadHandle for File {
    fn metadata(&self) -> std::io::Result<fs::Metadata> {
        File::metadata(self)
    }
}

fn export_snapshot_to_with_io<R: BlobReadHandle>(
    path: &Path,
    mut items: Vec<ClipboardItem>,
    blob_store: &ImageBlobStore,
    hook: impl FnOnce() -> anyhow::Result<()>,
    mut open_blob: impl FnMut(&Path) -> std::io::Result<R>,
    mut after_validate: impl FnMut(&Path) -> anyhow::Result<()>,
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
                blobs.push((archive_path, source_path, content_hash.to_string()));
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
            for (archive_path, source_path, expected_hash) in blobs {
                let mut file = open_blob(&source_path)
                    .with_context(|| format!("open image blob {}", source_path.display()))?;
                let metadata = file.metadata().with_context(|| {
                    format!("inspect open image blob {}", source_path.display())
                })?;
                anyhow::ensure!(
                    metadata.is_file(),
                    "image blob handle must be a regular file"
                );
                let expected_len = metadata.len();
                crate::blobs::validate_bmp_file_header(&mut file, expected_len)
                    .with_context(|| format!("validate image blob {}", source_path.display()))?;
                after_validate(&source_path)?;
                file.seek(SeekFrom::Start(0))
                    .with_context(|| format!("rewind image blob {}", source_path.display()))?;
                let written = write_blob_entry(&mut archive, &archive_path, &mut file)
                    .with_context(|| format!("archive image blob {}", source_path.display()))?;
                anyhow::ensure!(
                    written == expected_len,
                    "image blob length changed during export: {expected_hash}"
                );
                let final_len = file
                    .metadata()
                    .with_context(|| format!("reinspect image blob {}", source_path.display()))?
                    .len();
                anyhow::ensure!(
                    final_len == expected_len,
                    "image blob length changed during export: {expected_hash}"
                );
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};
    use crate::storage::repository::{ClipboardItem, ClipboardRepository};

    #[cfg(target_os = "windows")]
    use super::replace_target_with;
    use super::{
        export_snapshot_to_with_hook, export_snapshot_to_with_io, export_zip_to,
        export_zip_to_with_hook, finalize_temp_import_cleanup, import_backup_from_path,
        parse_backup_info_path, write_backup_atomically, write_backup_atomically_with_installer,
        write_blob_entry, ImportMode, MAX_MANIFEST_BYTES,
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

    fn opaque_image_item(
        store: &crate::blobs::store::ImageBlobStore,
        content_hash: &str,
        fill: u8,
    ) -> (ClipboardItem, std::path::PathBuf, Vec<u8>) {
        let source_path = store.blob_dir().join(format!("{content_hash}.bmp"));
        let mut bytes = opaque_bmp(64);
        bytes[14..].fill(fill);
        fs::write(&source_path, &bytes).expect("opaque image blob");
        let item = ClipboardItem {
            id: format!("item-{fill}"),
            hash: format!("record-{fill}"),
            item_type: "image".to_string(),
            content: None,
            content_path: Some(source_path.to_string_lossy().into_owned()),
            content_hash: Some(content_hash.to_string()),
            preview: format!("image-{fill}"),
            source_app: None,
            favorite: false,
            pinned: false,
            size_bytes: i64::try_from(bytes.len()).expect("image size"),
            created_at: 1,
            updated_at: 1,
        };
        (item, source_path, bytes)
    }

    struct TrackedFile {
        inner: fs::File,
        active: Arc<AtomicUsize>,
    }

    impl TrackedFile {
        fn new(inner: fs::File, active: Arc<AtomicUsize>, max_active: &AtomicUsize) -> Self {
            let active_count = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(active_count, Ordering::SeqCst);
            Self { inner, active }
        }
    }

    impl Drop for TrackedFile {
        fn drop(&mut self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Read for TrackedFile {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buffer)
        }
    }

    impl std::io::Seek for TrackedFile {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(position)
        }
    }

    impl super::BlobReadHandle for TrackedFile {
        fn metadata(&self) -> std::io::Result<fs::Metadata> {
            self.inner.metadata()
        }
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

    fn write_single_image_zip(
        path: &std::path::Path,
        id: &str,
        record_hash: &str,
        content_hash: &str,
        bmp: &[u8],
    ) {
        let archive_blob_path = format!("blobs/{content_hash}.bmp");
        let file = fs::File::create(path).expect("create ZIP");
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file("manifest.json", options)
            .expect("manifest entry");
        serde_json::to_writer(
            &mut archive,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-16T00:00:00Z",
                "item_count": 1,
                "items": [{
                    "id": id,
                    "hash": record_hash,
                    "item_type": "image",
                    "content": null,
                    "content_path": archive_blob_path,
                    "content_hash": content_hash,
                    "preview": "image",
                    "source_app": null,
                    "favorite": false,
                    "pinned": false,
                    "size_bytes": bmp.len(),
                    "created_at": 1,
                    "updated_at": 1
                }]
            }),
        )
        .expect("manifest");
        archive
            .start_file(&archive_blob_path, options)
            .expect("blob entry");
        archive.write_all(bmp).expect("blob bytes");
        archive.finish().expect("finish ZIP");
    }

    fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16 field"))
    }

    fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 field"))
    }

    fn standard_eocd_offset(bytes: &[u8]) -> usize {
        bytes
            .windows(4)
            .rposition(|window| window == b"PK\x05\x06")
            .expect("standard EOCD")
    }

    fn duplicate_raw_central_entry(path: &std::path::Path, duplicate_name: &str) {
        duplicate_raw_central_entry_as(path, duplicate_name, duplicate_name);
    }

    fn duplicate_raw_central_entry_as(
        path: &std::path::Path,
        duplicate_name: &str,
        replacement_name: &str,
    ) {
        assert_eq!(duplicate_name.len(), replacement_name.len());
        let mut bytes = fs::read(path).expect("read ZIP fixture");
        let eocd = standard_eocd_offset(&bytes);
        let central_start =
            usize::try_from(read_u32_le(&bytes, eocd + 16)).expect("central start fits usize");
        let mut cursor = central_start;
        let mut duplicate = None;
        while cursor < eocd {
            assert_eq!(&bytes[cursor..cursor + 4], b"PK\x01\x02");
            let name_len = usize::from(read_u16_le(&bytes, cursor + 28));
            let extra_len = usize::from(read_u16_le(&bytes, cursor + 30));
            let comment_len = usize::from(read_u16_le(&bytes, cursor + 32));
            let entry_end = cursor + 46 + name_len + extra_len + comment_len;
            if &bytes[cursor + 46..cursor + 46 + name_len] == duplicate_name.as_bytes() {
                duplicate = Some(bytes[cursor..entry_end].to_vec());
                break;
            }
            cursor = entry_end;
        }
        let mut duplicate = duplicate.expect("central entry to duplicate");
        duplicate[46..46 + replacement_name.len()].copy_from_slice(replacement_name.as_bytes());
        bytes.splice(eocd..eocd, duplicate.iter().copied());
        let new_eocd = eocd + duplicate.len();
        let count = read_u16_le(&bytes, new_eocd + 10)
            .checked_add(1)
            .expect("entry count");
        bytes[new_eocd + 8..new_eocd + 10].copy_from_slice(&count.to_le_bytes());
        bytes[new_eocd + 10..new_eocd + 12].copy_from_slice(&count.to_le_bytes());
        let central_size = read_u32_le(&bytes, new_eocd + 12)
            .checked_add(u32::try_from(duplicate.len()).expect("central entry length"))
            .expect("central size");
        bytes[new_eocd + 12..new_eocd + 16].copy_from_slice(&central_size.to_le_bytes());
        fs::write(path, bytes).expect("write duplicate central entry");
    }

    fn convert_standard_fixture_to_zip64(
        path: &std::path::Path,
        entry_count: u64,
        central_size: u64,
        central_offset: u64,
    ) {
        let mut bytes = fs::read(path).expect("read standard ZIP fixture");
        let eocd = standard_eocd_offset(&bytes);
        let mut standard_eocd = bytes[eocd..].to_vec();
        standard_eocd[8..12].fill(0xff);
        standard_eocd[12..20].fill(0xff);

        bytes.truncate(eocd);
        bytes.extend_from_slice(b"PK\x06\x06");
        bytes.extend_from_slice(&44u64.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&entry_count.to_le_bytes());
        bytes.extend_from_slice(&entry_count.to_le_bytes());
        bytes.extend_from_slice(&central_size.to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(eocd as u64).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&standard_eocd);
        fs::write(path, bytes).expect("write ZIP64 fixture");
    }

    fn standard_central_metadata(path: &std::path::Path) -> (u64, u64, u64) {
        let bytes = fs::read(path).expect("read standard ZIP metadata");
        let eocd = standard_eocd_offset(&bytes);
        (
            u64::from(read_u16_le(&bytes, eocd + 10)),
            u64::from(read_u32_le(&bytes, eocd + 12)),
            u64::from(read_u32_le(&bytes, eocd + 16)),
        )
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

        export_snapshot_to_with_io(
            &archive_path,
            vec![item],
            &store,
            || Ok(()),
            |path| fs::File::open(path),
            |_| {
                fs::rename(&source_path, &moved_original)?;
                fs::rename(&replacement_path, &source_path)?;
                Ok(())
            },
        )
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
    fn export_opens_at_most_one_blob_handle_at_a_time() {
        let root = temp_dir("one-open-blob");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let (first, _, _) = opaque_image_item(&store, &"3".repeat(64), 0x33);
        let (second, _, _) = opaque_image_item(&store, &"4".repeat(64), 0x44);
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let opener_active = Arc::clone(&active);
        let opener_max = Arc::clone(&max_active);

        export_snapshot_to_with_io(
            &root.join("backup.zip"),
            vec![first, second],
            &store,
            || Ok(()),
            move |path| {
                fs::File::open(path).map(|file| {
                    TrackedFile::new(file, Arc::clone(&opener_active), opener_max.as_ref())
                })
            },
            |_| Ok(()),
        )
        .expect("export");

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
        assert_eq!(active.load(Ordering::SeqCst), 0);

        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn export_rejects_blob_length_changes_after_handle_validation() {
        let root = temp_dir("blob-length-race");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let (item, source_path, original_bytes) = opaque_image_item(&store, &"5".repeat(64), 0x55);
        let target = root.join("backup.zip");
        let existing_target = b"existing complete backup";

        fs::write(&target, existing_target).expect("existing target");
        let error = export_snapshot_to_with_io(
            &target,
            vec![item.clone()],
            &store,
            || Ok(()),
            |path| fs::File::open(path),
            |path| {
                fs::OpenOptions::new().write(true).open(path)?.set_len(20)?;
                Ok(())
            },
        )
        .expect_err("truncate after validation must fail");
        assert!(
            format!("{error:#}").contains("blob length changed"),
            "unexpected truncate error: {error:#}"
        );
        assert_eq!(
            fs::read(&target).expect("target after truncate"),
            existing_target
        );

        fs::write(&source_path, &original_bytes).expect("restore source blob");
        fs::write(&target, existing_target).expect("restore existing target");
        let error = export_snapshot_to_with_io(
            &target,
            vec![item],
            &store,
            || Ok(()),
            |path| fs::File::open(path),
            |path| {
                let mut file = fs::OpenOptions::new().append(true).open(path)?;
                file.write_all(&[0x66; 16])?;
                file.flush()?;
                Ok(())
            },
        )
        .expect_err("append after validation must fail");
        assert!(
            format!("{error:#}").contains("blob length changed"),
            "unexpected append error: {error:#}"
        );
        assert_eq!(
            fs::read(&target).expect("target after append"),
            existing_target
        );

        let leftovers = fs::read_dir(&root)
            .expect("read root")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);

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
    fn export_orders_equal_rank_items_and_first_blob_references_by_id() {
        let root = temp_dir("deterministic-order");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        for (id, hash_byte) in [("c", 'c'), ("a", 'a'), ("b", 'b')] {
            let content_hash = hash_byte.to_string().repeat(64);
            let source_path = store.blob_dir().join(format!("{content_hash}.bmp"));
            fs::write(&source_path, opaque_bmp(64)).expect("source blob");
            repository
                .lock()
                .expect("repository lock")
                .insert_imported_item(&ClipboardItem {
                    id: id.to_string(),
                    hash: format!("record-{id}"),
                    item_type: "image".to_string(),
                    content: None,
                    content_path: Some(source_path.to_string_lossy().into_owned()),
                    content_hash: Some(content_hash),
                    preview: id.to_string(),
                    source_app: None,
                    favorite: false,
                    pinned: false,
                    size_bytes: 78,
                    created_at: 1,
                    updated_at: 1,
                })
                .expect("insert equal-rank item");
        }

        let expected_ids = ["a", "b", "c"];
        let expected_entries = vec![
            "manifest.json".to_string(),
            format!("blobs/{}.bmp", "a".repeat(64)),
            format!("blobs/{}.bmp", "b".repeat(64)),
            format!("blobs/{}.bmp", "c".repeat(64)),
        ];
        for run in 0..3 {
            let archive_path = root.join(format!("backup-{run}.zip"));
            export_zip_to(&archive_path, &repository, &store).expect("deterministic export");
            let file = fs::File::open(&archive_path).expect("archive");
            let mut archive = zip::ZipArchive::new(file).expect("ZIP");
            let manifest: serde_json::Value =
                serde_json::from_reader(archive.by_name("manifest.json").expect("manifest"))
                    .expect("manifest JSON");
            let ids = manifest["items"]
                .as_array()
                .expect("manifest items")
                .iter()
                .map(|item| item["id"].as_str().expect("item id"))
                .collect::<Vec<_>>();
            assert_eq!(ids, expected_ids);
            let entries = archive.file_names().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(entries, expected_entries);
        }

        drop(repository);
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
        let (writer_attempted_tx, writer_attempted_rx) = mpsc::channel();
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
            writer_attempted_tx
                .send(())
                .expect("signal writer write-lease attempt");
            writer_store.with_write(|_, _| {
                writer_entered_tx.send(()).expect("signal writer");
                Ok(())
            })
        });
        writer_attempted_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer attempted write lease");
        assert!(matches!(
            writer_entered_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
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
    fn atomic_install_failure_preserves_existing_and_absent_targets() {
        for existing in [false, true] {
            let root = temp_dir(if existing {
                "install-failure-existing"
            } else {
                "install-failure-new"
            });
            fs::create_dir_all(&root).expect("root");
            let target = root.join("backup.zip");
            if existing {
                fs::write(&target, b"existing complete backup").expect("existing target");
            }

            let error = write_backup_atomically_with_installer(
                &target,
                |file| {
                    file.write_all(b"complete new backup")?;
                    Ok(())
                },
                |_, _| Err(anyhow::anyhow!("injected install failure")),
            )
            .expect_err("install failure must propagate");

            assert!(format!("{error:#}").contains("injected install failure"));
            if existing {
                assert_eq!(
                    fs::read(&target).expect("existing target after failure"),
                    b"existing complete backup"
                );
            } else {
                assert!(!target.exists());
            }
            let leftovers = fs::read_dir(&root)
                .expect("read root")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
                .count();
            assert_eq!(leftovers, 0);
            fs::remove_dir_all(root).expect("cleanup");
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_atomic_install_requests_replace_and_write_through() {
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let root = temp_dir("install-flags");
        fs::create_dir_all(&root).expect("root");
        let temp = root.join("backup.tmp");
        let target = root.join("backup.zip");
        let observed = AtomicUsize::new(0);

        let error = replace_target_with(&temp, &target, |source, destination, flags| {
            assert_eq!(source, temp);
            assert_eq!(destination, target);
            observed.store(flags as usize, Ordering::SeqCst);
            Err(std::io::Error::other("injected move failure"))
        })
        .expect_err("injected move must fail");

        assert!(format!("{error:#}").contains("injected move failure"));
        assert_eq!(
            observed.load(Ordering::SeqCst) as u32,
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
        );
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
            "CON.foo.bmp",
            "COM1.foo.bmp",
            "LPT9.backup.bmp",
            "CLOCK$.log.bmp",
            "CON .foo.bmp",
            "COM1..backup.bmp",
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

        write_legacy_json(
            &path,
            "1.0",
            0,
            Vec::new(),
            vec![legacy_blob("console.foo.bmp", "AA==")],
        );
        assert!(
            parse_backup_info_path(&path).is_ok(),
            "ordinary names with a device prefix must remain valid"
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
    fn parse_backup_info_rejects_duplicate_raw_zip_entry_names() {
        let root = temp_dir("parse-raw-duplicates");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("backup.zip");

        write_test_zip(
            &path,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-15T00:00:00Z",
                "item_count": 0,
                "items": []
            }),
            &[],
        );
        duplicate_raw_central_entry(&path, "manifest.json");
        assert!(
            parse_backup_info_path(&path).is_err(),
            "duplicate raw manifest entries must fail"
        );

        let hash = "0".repeat(64);
        let blob_name = format!("blobs/{hash}.bmp");
        write_test_zip(
            &path,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-15T00:00:00Z",
                "item_count": 1,
                "items": [manifest_image(&blob_name, &hash)]
            }),
            &[&blob_name],
        );
        duplicate_raw_central_entry(&path, &blob_name);
        assert!(
            parse_backup_info_path(&path).is_err(),
            "duplicate raw blob entries must fail"
        );

        write_test_zip(
            &path,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-15T00:00:00Z",
                "item_count": 1,
                "items": [manifest_image(&blob_name, &hash)]
            }),
            &[&blob_name],
        );
        let backslash_name = blob_name.replace('/', "\\");
        duplicate_raw_central_entry_as(&path, &blob_name, &backslash_name);
        assert!(
            parse_backup_info_path(&path).is_err(),
            "normalized duplicate raw blob entries must fail"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_preflights_standard_zip_counts_and_bounds() {
        let root = temp_dir("parse-standard-preflight");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("backup.zip");
        let manifest = serde_json::json!({
            "version": 2,
            "exported_at": "2026-07-15T00:00:00Z",
            "item_count": 0,
            "items": []
        });

        write_test_zip(&path, &manifest, &[]);
        let mut bytes = fs::read(&path).expect("read count fixture");
        let eocd = standard_eocd_offset(&bytes);
        bytes[eocd + 8..eocd + 10].copy_from_slice(&60_000u16.to_le_bytes());
        bytes[eocd + 10..eocd + 12].copy_from_slice(&60_000u16.to_le_bytes());
        fs::write(&path, bytes).expect("patch entry count");
        let error = parse_backup_info_path(&path).expect_err("impossible entry count must fail");
        assert!(format!("{error:#}").contains("entry count exceeds declared central directory"));

        write_test_zip(&path, &manifest, &[]);
        let mut bytes = fs::read(&path).expect("read central size fixture");
        let eocd = standard_eocd_offset(&bytes);
        bytes[eocd + 12..eocd + 16].copy_from_slice(&(64u32 * 1024 * 1024 + 1).to_le_bytes());
        fs::write(&path, bytes).expect("patch central size");
        let error =
            parse_backup_info_path(&path).expect_err("oversize central directory must fail");
        assert!(format!("{error:#}").contains("central directory exceeds"));

        write_test_zip(&path, &manifest, &[]);
        let mut bytes = fs::read(&path).expect("read central offset fixture");
        let eocd = standard_eocd_offset(&bytes);
        bytes[eocd + 16..eocd + 20].copy_from_slice(&0xffff_fffeu32.to_le_bytes());
        fs::write(&path, bytes).expect("patch central offset");
        let error =
            parse_backup_info_path(&path).expect_err("out-of-range central offset must fail");
        assert!(format!("{error:#}").contains("central directory is outside the ZIP file"));

        write_test_zip(&path, &manifest, &[]);
        let mut bytes = fs::read(&path).expect("read multi-disk fixture");
        let eocd = standard_eocd_offset(&bytes);
        let central_start = usize::try_from(read_u32_le(&bytes, eocd + 16)).expect("central start");
        bytes[central_start + 34..central_start + 36].copy_from_slice(&1u16.to_le_bytes());
        fs::write(&path, bytes).expect("patch central disk number");
        let error = parse_backup_info_path(&path).expect_err("multi-disk entry must fail");
        assert!(format!("{error:#}").contains("multi-disk ZIP"));

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_backup_info_preflights_zip64_counts_and_bounds() {
        let root = temp_dir("parse-zip64-preflight");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("backup.zip");
        let manifest = serde_json::json!({
            "version": 2,
            "exported_at": "2026-07-15T00:00:00Z",
            "item_count": 0,
            "items": []
        });

        write_test_zip(&path, &manifest, &[]);
        let (count, central_size, central_offset) = standard_central_metadata(&path);
        convert_standard_fixture_to_zip64(&path, count, central_size, central_offset);
        let info = parse_backup_info_path(&path).expect("valid synthetic ZIP64 metadata");
        assert_eq!(info.version, "2");

        write_test_zip(&path, &manifest, &[]);
        let (_, central_size, central_offset) = standard_central_metadata(&path);
        convert_standard_fixture_to_zip64(&path, 100_002, central_size, central_offset);
        let error = parse_backup_info_path(&path).expect_err("ZIP64 entry bomb must fail");
        assert!(format!("{error:#}").contains("ZIP entry count exceeds"));

        write_test_zip(&path, &manifest, &[]);
        let (count, _, _) = standard_central_metadata(&path);
        convert_standard_fixture_to_zip64(&path, count, 100, u64::MAX - 10);
        let error = parse_backup_info_path(&path).expect_err("overflowing ZIP64 bounds must fail");
        assert!(format!("{error:#}").contains("central directory bounds overflow"));

        write_test_zip(&path, &manifest, &[]);
        let (count, _, central_offset) = standard_central_metadata(&path);
        convert_standard_fixture_to_zip64(&path, count, 64u64 * 1024 * 1024 + 1, central_offset);
        let error = parse_backup_info_path(&path).expect_err("oversize ZIP64 central must fail");
        assert!(format!("{error:#}").contains("central directory exceeds"));

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

    #[test]
    fn import_legacy_json_normalizes_image_into_canonical_stage_pipeline() {
        use base64::Engine as _;

        let root = temp_dir("import-legacy-image");
        fs::create_dir_all(&root).expect("root");
        let source_stage = root.join("source-stage");
        let source = crate::blobs::image::stage_dib(&source_stage, dib32([30, 20, 10, 255]))
            .expect("source image");
        let content_hash = source.content_hash().to_string();
        let bmp = fs::read(source.bmp_path()).expect("source BMP");
        let backup_path = root.join("legacy.json");
        write_legacy_json(
            &backup_path,
            "1.0",
            1,
            vec![legacy_item("image", Some("legacy.bmp"))],
            vec![legacy_blob(
                "legacy.bmp",
                &base64::engine::general_purpose::STANDARD.encode(&bmp),
            )],
        );
        fs::remove_dir_all(&source_stage).expect("source stage cleanup");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");

        let result = import_backup_from_path(
            &backup_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety-backups"),
        )
        .expect("legacy staged import");

        assert_eq!(result.imported, 1);
        let imported = repository
            .lock()
            .expect("repository lock")
            .get_item("legacy-item")
            .expect("query")
            .expect("legacy item");
        assert_eq!(
            imported.content_hash.as_deref(),
            Some(content_hash.as_str())
        );
        assert_eq!(
            imported.content_path.as_deref(),
            Some(
                crate::blobs::image::canonical_bmp_path(store.blob_dir(), &content_hash)
                    .expect("canonical path")
                    .to_string_lossy()
                    .as_ref()
            )
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
    fn committed_legacy_import_is_not_reported_failed_when_temp_cleanup_fails() {
        let expected = super::ImportResult {
            imported: 2,
            skipped: 1,
            safety_backup: None,
        };

        let result = finalize_temp_import_cleanup(
            Ok(expected.clone()),
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "locked temp file",
            )),
            std::path::Path::new("legacy-import.zip"),
        )
        .expect("committed import result must win over temp cleanup failure");

        assert_eq!(result, expected);
    }

    #[test]
    fn import_zip_v2_inserts_text_and_fts_in_one_pipeline() {
        let root = temp_dir("import-zip-text");
        fs::create_dir_all(&root).expect("root");
        let archive_path = root.join("backup.zip");
        let item = serde_json::json!({
            "id": "zip-text",
            "hash": "zip-text-hash",
            "item_type": "text",
            "content": "transactional searchable content",
            "content_path": null,
            "content_hash": null,
            "preview": "transactional searchable content",
            "source_app": "fixture",
            "favorite": true,
            "pinned": false,
            "size_bytes": 32,
            "created_at": 1,
            "updated_at": 2
        });
        write_test_zip(
            &archive_path,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-16T00:00:00Z",
                "item_count": 1,
                "items": [item]
            }),
            &[],
        );
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");

        let result = import_backup_from_path(
            &archive_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety-backups"),
        )
        .expect("transactional ZIP import");

        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 0);
        {
            let repository_guard = repository.lock().expect("repository lock");
            assert_eq!(
                repository_guard
                    .get_item("zip-text")
                    .expect("get imported")
                    .expect("imported item")
                    .content
                    .as_deref(),
                Some("transactional searchable content")
            );
            assert_eq!(
                repository_guard
                    .search(
                        "searchable".to_string(),
                        crate::storage::repository::SearchFilters {
                            item_type: None,
                            favorites_only: false,
                        },
                        10,
                        None
                    )
                    .expect("FTS search")
                    .len(),
                1
            );
        }
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn import_legacy_json_uses_the_same_normalized_pipeline() {
        use base64::Engine as _;

        let root = temp_dir("import-legacy-normalized");
        fs::create_dir_all(&root).expect("root");
        let dib = dib32([30, 20, 10, 255]);
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(
            &u32::try_from(14 + dib.len())
                .expect("BMP size")
                .to_le_bytes(),
        );
        bmp.extend_from_slice(&[0u8; 4]);
        bmp.extend_from_slice(&14u32.to_le_bytes());
        bmp.extend_from_slice(&dib);
        let archive_path = root.join("legacy.json");
        fs::write(
            &archive_path,
            serde_json::to_vec(&serde_json::json!({
                "metadata": {
                    "version": "1.0",
                    "created_at": "2026-07-16T00:00:00Z",
                    "item_count": 2
                },
                "items": [
                    {
                        "id": "legacy-text",
                        "hash": "legacy-text-hash",
                        "item_type": "text",
                        "content": "legacy searchable content",
                        "content_path": null,
                        "content_hash": null,
                        "preview": "legacy searchable content",
                        "source_app": "fixture",
                        "favorite": false,
                        "pinned": false,
                        "size_bytes": 26,
                        "created_at": 1,
                        "updated_at": 1
                    },
                    {
                        "id": "legacy-image",
                        "hash": "legacy-image-hash",
                        "item_type": "image",
                        "content": null,
                        "content_path": "image.bmp",
                        "content_hash": null,
                        "preview": "legacy image",
                        "source_app": null,
                        "favorite": false,
                        "pinned": false,
                        "size_bytes": bmp.len(),
                        "created_at": 2,
                        "updated_at": 2
                    }
                ],
                "blobs": [{
                    "item_id": "legacy-image",
                    "filename": "image.bmp",
                    "data_base64": base64::engine::general_purpose::STANDARD.encode(&bmp)
                }]
            }))
            .expect("legacy JSON"),
        )
        .expect("write legacy JSON");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");

        let result = import_backup_from_path(
            &archive_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety-backups"),
        )
        .expect("legacy import");

        assert_eq!(result.imported, 2);
        let text = repository
            .lock()
            .expect("repository lock")
            .get_item("legacy-text")
            .expect("text query")
            .expect("text item");
        assert_eq!(text.content.as_deref(), Some("legacy searchable content"));
        let image = repository
            .lock()
            .expect("repository lock")
            .get_item("legacy-image")
            .expect("image query")
            .expect("image item");
        let content_hash = image.content_hash.expect("canonical image hash");
        assert_eq!(image.hash, format!("image:{content_hash}"));
        assert!(
            crate::blobs::image::canonical_bmp_path(store.blob_dir(), &content_hash)
                .expect("canonical BMP")
                .is_file()
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
    fn import_zip_v2_stages_image_and_installs_canonical_files() {
        let root = temp_dir("import-zip-image");
        fs::create_dir_all(&root).expect("root");
        let source_stage = root.join("source-stage");
        let source = crate::blobs::image::stage_dib(&source_stage, dib32([30, 20, 10, 255]))
            .expect("source image");
        let content_hash = source.content_hash().to_string();
        let bmp = fs::read(source.bmp_path()).expect("source BMP");
        let archive_path = root.join("backup.zip");
        let archive_blob_path = format!("blobs/{content_hash}.bmp");
        let item = serde_json::json!({
            "id": "zip-image",
            "hash": "zip-image-record-hash",
            "item_type": "image",
            "content": null,
            "content_path": archive_blob_path,
            "content_hash": content_hash,
            "preview": "1 x 1 image",
            "source_app": "fixture",
            "favorite": false,
            "pinned": true,
            "size_bytes": bmp.len(),
            "created_at": 3,
            "updated_at": 4
        });
        let file = fs::File::create(&archive_path).expect("create ZIP");
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file("manifest.json", options)
            .expect("manifest entry");
        serde_json::to_writer(
            &mut archive,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-16T00:00:00Z",
                "item_count": 1,
                "items": [item]
            }),
        )
        .expect("manifest");
        archive
            .start_file(&archive_blob_path, options)
            .expect("blob entry");
        archive.write_all(&bmp).expect("blob bytes");
        archive.finish().expect("finish ZIP");
        fs::remove_dir_all(&source_stage).expect("source stage cleanup");

        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");

        let result = import_backup_from_path(
            &archive_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety-backups"),
        )
        .expect("transactional image import");

        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 0);
        let expected_bmp = crate::blobs::image::canonical_bmp_path(store.blob_dir(), &content_hash)
            .expect("canonical BMP");
        let expected_thumbnail =
            crate::blobs::image::canonical_thumbnail_path(store.blob_dir(), &content_hash)
                .expect("canonical thumbnail");
        assert!(expected_bmp.is_file());
        assert!(expected_thumbnail.is_file());
        assert!(fs::read_dir(store.stage_dir())
            .expect("stage directory")
            .next()
            .is_none());
        let imported = repository
            .lock()
            .expect("repository lock")
            .get_item("zip-image")
            .expect("get imported")
            .expect("imported image");
        assert_eq!(
            imported.content_hash.as_deref(),
            Some(content_hash.as_str())
        );
        assert_eq!(imported.content_path.as_deref(), expected_bmp.to_str());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn import_zip_v2_semantic_hash_mismatch_cleans_stage_without_mutation() {
        let root = temp_dir("import-zip-image-mismatch");
        fs::create_dir_all(&root).expect("root");
        let source_stage = root.join("source-stage");
        let source = crate::blobs::image::stage_dib(&source_stage, dib32([30, 20, 10, 255]))
            .expect("source image");
        let bmp = fs::read(source.bmp_path()).expect("source BMP");
        let wrong_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let archive_blob_path = format!("blobs/{wrong_hash}.bmp");
        let archive_path = root.join("backup.zip");
        let file = fs::File::create(&archive_path).expect("create ZIP");
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file("manifest.json", options)
            .expect("manifest entry");
        serde_json::to_writer(
            &mut archive,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-16T00:00:00Z",
                "item_count": 1,
                "items": [{
                    "id": "mismatched-image",
                    "hash": "mismatched-record-hash",
                    "item_type": "image",
                    "content": null,
                    "content_path": archive_blob_path,
                    "content_hash": wrong_hash,
                    "preview": "mismatch",
                    "source_app": null,
                    "favorite": false,
                    "pinned": false,
                    "size_bytes": bmp.len(),
                    "created_at": 1,
                    "updated_at": 1
                }]
            }),
        )
        .expect("manifest");
        archive
            .start_file(&archive_blob_path, options)
            .expect("blob entry");
        archive.write_all(&bmp).expect("blob bytes");
        archive.finish().expect("finish ZIP");
        fs::remove_dir_all(&source_stage).expect("source stage cleanup");

        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");

        import_backup_from_path(
            &archive_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety-backups"),
        )
        .expect_err("semantic mismatch must fail");

        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item("mismatched-image")
            .expect("query")
            .is_none());
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
    fn import_zip_v2_rejects_projected_usage_above_managed_quota() {
        let root = temp_dir("import-zip-quota");
        fs::create_dir_all(&root).expect("root");
        let source_stage = root.join("source-stage");
        let source = crate::blobs::image::stage_dib(&source_stage, dib32([30, 20, 10, 255]))
            .expect("source image");
        let content_hash = source.content_hash().to_string();
        let bmp = fs::read(source.bmp_path()).expect("source BMP");
        let archive_path = root.join("backup.zip");
        write_single_image_zip(
            &archive_path,
            "quota-image",
            "quota-record-hash",
            &content_hash,
            &bmp,
        );
        fs::remove_dir_all(&source_stage).expect("source stage cleanup");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let filler = store.blob_dir().join("quota-filler");
        fs::File::create(&filler)
            .expect("filler")
            .set_len(crate::storage::capacity::MANAGED_BLOB_QUOTA)
            .expect("sparse quota filler");

        import_backup_from_path(
            &archive_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety-backups"),
        )
        .expect_err("projected managed usage must reject import");

        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item("quota-image")
            .expect("query")
            .is_none());
        assert!(filler.is_file());
        assert!(fs::read_dir(store.stage_dir())
            .expect("stage directory")
            .next()
            .is_none());
        assert!(
            !crate::blobs::image::canonical_bmp_path(store.blob_dir(), &content_hash)
                .expect("canonical BMP")
                .exists()
        );
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn overwrite_import_aborts_before_mutation_when_safety_backup_fails() {
        let root = temp_dir("import-overwrite-safety-failure");
        fs::create_dir_all(&root).expect("root");
        let archive_path = root.join("backup.zip");
        write_test_zip(
            &archive_path,
            &serde_json::json!({
                "version": 2,
                "exported_at": "2026-07-16T00:00:00Z",
                "item_count": 1,
                "items": [{
                    "id": "incoming",
                    "hash": "incoming-hash",
                    "item_type": "text",
                    "content": "incoming",
                    "content_path": null,
                    "content_hash": null,
                    "preview": "incoming",
                    "source_app": null,
                    "favorite": false,
                    "pinned": false,
                    "size_bytes": 8,
                    "created_at": 2,
                    "updated_at": 2
                }]
            }),
            &[],
        );
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        repository
            .lock()
            .expect("repository lock")
            .insert_imported_item(&ClipboardItem {
                id: "existing".to_string(),
                hash: "existing-hash".to_string(),
                item_type: "text".to_string(),
                content: Some("keep me".to_string()),
                content_path: None,
                content_hash: None,
                preview: "keep me".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 7,
                created_at: 1,
                updated_at: 1,
            })
            .expect("existing item");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let invalid_safety_dir = root.join("not-a-directory");
        fs::write(&invalid_safety_dir, b"file").expect("invalid safety directory");

        import_backup_from_path(
            &archive_path,
            ImportMode::Overwrite,
            &repository,
            &store,
            &invalid_safety_dir,
        )
        .expect_err("safety backup failure must abort overwrite");

        let repository_guard = repository.lock().expect("repository lock");
        assert!(repository_guard
            .get_item("existing")
            .expect("existing query")
            .is_some());
        assert!(repository_guard
            .get_item("incoming")
            .expect("incoming query")
            .is_none());
        drop(repository_guard);
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn overwrite_import_projects_old_cleanup_and_writes_valid_safety_backup() {
        let root = temp_dir("import-overwrite-projection");
        fs::create_dir_all(&root).expect("root");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let mut old_dib = vec![0u8; 40];
        old_dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        old_dib[4..8].copy_from_slice(&100i32.to_le_bytes());
        old_dib[8..12].copy_from_slice(&(-100i32).to_le_bytes());
        old_dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        old_dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        old_dib[20..24].copy_from_slice(&40_000u32.to_le_bytes());
        old_dib.extend(std::iter::repeat([30, 20, 10, 255]).take(10_000).flatten());
        let old = store
            .install_staged_with(
                crate::blobs::image::stage_dib(store.stage_dir(), old_dib).expect("old stage"),
                |installed| Ok(installed.clone()),
                |_| Ok(false),
            )
            .expect("old install");
        repository
            .lock()
            .expect("repository lock")
            .insert_imported_item(&ClipboardItem {
                id: "old-image".to_string(),
                hash: "old-record-hash".to_string(),
                item_type: "image".to_string(),
                content: None,
                content_path: Some(old.bmp_path().to_string_lossy().into_owned()),
                content_hash: Some(old.content_hash().to_string()),
                preview: "old".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: i64::try_from(fs::metadata(old.bmp_path()).expect("old BMP").len())
                    .expect("old size"),
                created_at: 1,
                updated_at: 1,
            })
            .expect("old row");
        let old_total = fs::metadata(old.bmp_path()).expect("old BMP").len()
            + fs::metadata(old.thumbnail_path())
                .expect("old thumbnail")
                .len();
        fs::File::create(store.blob_dir().join("quota-filler"))
            .expect("filler")
            .set_len(crate::storage::capacity::MANAGED_BLOB_QUOTA - old_total)
            .expect("sparse filler");

        let source_stage = root.join("source-stage");
        let incoming = crate::blobs::image::stage_dib(&source_stage, dib32([31, 20, 10, 255]))
            .expect("incoming source");
        let incoming_hash = incoming.content_hash().to_string();
        let incoming_bmp = fs::read(incoming.bmp_path()).expect("incoming BMP");
        let archive_path = root.join("backup.zip");
        write_single_image_zip(
            &archive_path,
            "new-image",
            "new-record-hash",
            &incoming_hash,
            &incoming_bmp,
        );
        fs::remove_dir_all(&source_stage).expect("source stage cleanup");

        let result = import_backup_from_path(
            &archive_path,
            ImportMode::Overwrite,
            &repository,
            &store,
            &root.join("safety"),
        )
        .expect("overwrite fits after projected cleanup");

        let safety = result.safety_backup.expect("safety backup path");
        assert_eq!(
            parse_backup_info_path(&safety)
                .expect("valid safety ZIP")
                .item_count,
            1
        );
        let repository_guard = repository.lock().expect("repository lock");
        assert!(repository_guard
            .get_item("old-image")
            .expect("old query")
            .is_none());
        assert!(repository_guard
            .get_item("new-image")
            .expect("new query")
            .is_some());
        assert!(repository_guard
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository_guard);
        assert!(!old.bmp_path().exists());
        assert!(!old.thumbnail_path().exists());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn merge_skip_cross_type_hash_removes_unreferenced_image_install() {
        let root = temp_dir("import-merge-cross-type-skip");
        fs::create_dir_all(&root).expect("root");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        repository
            .lock()
            .expect("repository lock")
            .insert_imported_item(&ClipboardItem {
                id: "existing-text".to_string(),
                hash: "shared-record-hash".to_string(),
                item_type: "text".to_string(),
                content: Some("existing".to_string()),
                content_path: None,
                content_hash: None,
                preview: "existing".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 8,
                created_at: 1,
                updated_at: 1,
            })
            .expect("existing text");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let source_stage = root.join("source-stage");
        let source = crate::blobs::image::stage_dib(&source_stage, dib32([30, 20, 10, 255]))
            .expect("source image");
        let content_hash = source.content_hash().to_string();
        let bmp = fs::read(source.bmp_path()).expect("source BMP");
        let archive_path = root.join("backup.zip");
        write_single_image_zip(
            &archive_path,
            "skipped-image",
            "shared-record-hash",
            &content_hash,
            &bmp,
        );
        fs::remove_dir_all(&source_stage).expect("source stage cleanup");

        let result = import_backup_from_path(
            &archive_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety"),
        )
        .expect("merge skip");

        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 1);
        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item("skipped-image")
            .expect("query")
            .is_none());
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
    fn merge_quota_projection_excludes_cross_type_duplicate_images() {
        let root = temp_dir("import-merge-duplicate-quota");
        fs::create_dir_all(&root).expect("root");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        repository
            .lock()
            .expect("repository lock")
            .insert_imported_item(&ClipboardItem {
                id: "existing-text".to_string(),
                hash: "shared-record-hash".to_string(),
                item_type: "text".to_string(),
                content: Some("existing".to_string()),
                content_path: None,
                content_hash: None,
                preview: "existing".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 8,
                created_at: 1,
                updated_at: 1,
            })
            .expect("existing text");
        let store =
            crate::blobs::store::ImageBlobStore::new(root.join("blobs"), root.join("stage"))
                .expect("store");
        let filler = store.blob_dir().join("quota-filler");
        fs::File::create(&filler)
            .expect("filler")
            .set_len(crate::storage::capacity::MANAGED_BLOB_QUOTA)
            .expect("sparse quota filler");
        let source_stage = root.join("source-stage");
        let source = crate::blobs::image::stage_dib(&source_stage, dib32([30, 20, 10, 255]))
            .expect("source image");
        let content_hash = source.content_hash().to_string();
        let bmp = fs::read(source.bmp_path()).expect("source BMP");
        let archive_path = root.join("backup.zip");
        write_single_image_zip(
            &archive_path,
            "skipped-image",
            "shared-record-hash",
            &content_hash,
            &bmp,
        );
        fs::remove_dir_all(&source_stage).expect("source stage cleanup");

        let result = import_backup_from_path(
            &archive_path,
            ImportMode::Merge,
            &repository,
            &store,
            &root.join("safety"),
        )
        .expect("duplicate image should not consume projected quota");

        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 1);
        assert!(filler.is_file());
        assert!(
            !crate::blobs::image::canonical_bmp_path(store.blob_dir(), &content_hash)
                .expect("canonical BMP")
                .exists()
        );
        assert!(fs::read_dir(store.stage_dir())
            .expect("stage directory")
            .next()
            .is_none());
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
