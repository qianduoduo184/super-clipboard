use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use ::image::{DynamicImage, RgbaImage};
use anyhow::{anyhow, ensure, Context};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageIdentity {
    pub content_hash: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedImage {
    content_hash: String,
    stage_dir: PathBuf,
    bmp_path: PathBuf,
    thumbnail_path: PathBuf,
    bmp_size: u64,
    thumbnail_size: u64,
}

impl StagedImage {
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn stage_dir(&self) -> &Path {
        &self.stage_dir
    }

    pub fn bmp_path(&self) -> &Path {
        &self.bmp_path
    }

    pub fn thumbnail_path(&self) -> &Path {
        &self.thumbnail_path
    }

    pub fn bmp_size(&self) -> u64 {
        self.bmp_size
    }

    pub fn thumbnail_size(&self) -> u64 {
        self.thumbnail_size
    }
}

pub fn image_identity_from_dib(dib: &[u8]) -> anyhow::Result<ImageIdentity> {
    let bmp = super::bmp_file_from_dib(dib).context("wrap DIB as BMP")?;
    let rgba = ::image::load_from_memory_with_format(&bmp, ::image::ImageFormat::Bmp)
        .context("decode DIB pixels")?
        .into_rgba8();
    Ok(image_identity_from_rgba(&rgba))
}

pub fn canonical_bmp_path(blob_dir: &Path, content_hash: &str) -> anyhow::Result<PathBuf> {
    canonical_image_path(blob_dir, content_hash, ".bmp")
}

pub fn canonical_thumbnail_path(blob_dir: &Path, content_hash: &str) -> anyhow::Result<PathBuf> {
    canonical_image_path(blob_dir, content_hash, ".thumb.png")
}

pub fn stage_dib(stage_root: &Path, dib: Vec<u8>) -> anyhow::Result<StagedImage> {
    fs::create_dir_all(stage_root)
        .with_context(|| format!("create stage root {}", stage_root.display()))?;
    let stage_id = Uuid::new_v4().to_string();
    let stage_dir = stage_root.join(&stage_id);
    fs::create_dir(&stage_dir)
        .with_context(|| format!("create image stage {}", stage_dir.display()))?;

    match stage_dib_in(&stage_dir, &stage_id, dib) {
        Ok(staged) => Ok(staged),
        Err(stage_error) => Err(clean_stage_after_error(&stage_dir, stage_error)),
    }
}

fn stage_dib_in(stage_dir: &Path, stage_id: &str, dib: Vec<u8>) -> anyhow::Result<StagedImage> {
    let raw_bmp_path = stage_dir.join(format!("{stage_id}.bmp"));
    let header = bmp_header(&dib)?;
    let mut bmp_file = File::create(&raw_bmp_path)
        .with_context(|| format!("create staged BMP {}", raw_bmp_path.display()))?;
    bmp_file.write_all(&header)?;
    bmp_file.write_all(&dib)?;
    bmp_file.flush()?;
    bmp_file.sync_all()?;
    let bmp_size = bmp_file.metadata()?.len();
    drop(bmp_file);
    drop(dib);

    let rgba = decode_rgba(&raw_bmp_path)
        .with_context(|| format!("decode staged BMP {}", raw_bmp_path.display()))?;
    let identity = image_identity_from_rgba(&rgba);
    let bmp_path = canonical_bmp_path(stage_dir, &identity.content_hash)?;
    fs::rename(&raw_bmp_path, &bmp_path).with_context(|| {
        format!(
            "name staged BMP {} as {}",
            raw_bmp_path.display(),
            bmp_path.display()
        )
    })?;

    let thumbnail_path = canonical_thumbnail_path(stage_dir, &identity.content_hash)?;
    let thumbnail = thumbnail_from_rgba(rgba);
    write_png_flushed(&thumbnail_path, &thumbnail)?;
    let thumbnail_size = fs::metadata(&thumbnail_path)?.len();

    Ok(StagedImage {
        content_hash: identity.content_hash,
        stage_dir: stage_dir.canonicalize()?,
        bmp_path,
        thumbnail_path,
        bmp_size,
        thumbnail_size,
    })
}

pub(crate) fn decoded_thumbnail_for_hash(
    bmp_path: &Path,
    expected_hash: &str,
) -> anyhow::Result<RgbaImage> {
    validate_content_hash(expected_hash)?;
    let rgba = decode_rgba(bmp_path)
        .with_context(|| format!("decode BMP for validation {}", bmp_path.display()))?;
    let actual = image_identity_from_rgba(&rgba);
    ensure!(
        actual.content_hash == expected_hash,
        "BMP semantic hash mismatch for {}: expected {}, got {}",
        bmp_path.display(),
        expected_hash,
        actual.content_hash
    );
    Ok(thumbnail_from_rgba(rgba))
}

pub(crate) fn validate_thumbnail(
    thumbnail_path: &Path,
    expected: &RgbaImage,
) -> anyhow::Result<()> {
    let actual = decode_rgba(thumbnail_path).with_context(|| {
        format!(
            "decode thumbnail for validation {}",
            thumbnail_path.display()
        )
    })?;
    ensure!(
        actual.dimensions() == expected.dimensions() && actual.as_raw() == expected.as_raw(),
        "thumbnail content mismatch for {}",
        thumbnail_path.display()
    );
    Ok(())
}

fn canonical_image_path(root: &Path, content_hash: &str, suffix: &str) -> anyhow::Result<PathBuf> {
    validate_content_hash(content_hash)?;
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize image root {}", root.display()))?;
    ensure!(
        root.is_dir(),
        "image root is not a directory: {}",
        root.display()
    );
    let path = root.join(format!("{content_hash}{suffix}"));
    ensure!(
        path.starts_with(&root) && path.parent() == Some(root.as_path()),
        "canonical image path escapes root {}",
        root.display()
    );
    Ok(path)
}

fn validate_content_hash(content_hash: &str) -> anyhow::Result<()> {
    ensure!(
        content_hash.len() == 64
            && content_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid image content hash"
    );
    Ok(())
}

fn image_identity_from_rgba(rgba: &RgbaImage) -> ImageIdentity {
    let width = rgba.width();
    let height = rgba.height();
    let mut hasher = Sha256::new();
    hasher.update(b"SCIMG1");
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    hasher.update(rgba.as_raw());
    let digest = hasher.finalize();
    let mut content_hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut content_hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    ImageIdentity {
        content_hash,
        width,
        height,
    }
}

fn decode_rgba(path: &Path) -> anyhow::Result<RgbaImage> {
    Ok(::image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?
        .into_rgba8())
}

fn thumbnail_from_rgba(rgba: RgbaImage) -> RgbaImage {
    DynamicImage::ImageRgba8(rgba)
        .thumbnail(320, 320)
        .into_rgba8()
}

fn write_png_flushed(path: &Path, rgba: &RgbaImage) -> anyhow::Result<()> {
    let file =
        File::create(path).with_context(|| format!("create thumbnail {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    DynamicImage::ImageRgba8(rgba.clone()).write_to(&mut writer, ::image::ImageFormat::Png)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn clean_stage_after_error(stage_dir: &Path, stage_error: anyhow::Error) -> anyhow::Error {
    match fs::remove_dir_all(stage_dir) {
        Ok(()) => stage_error,
        Err(cleanup_error) => anyhow!(
            "{stage_error:#}; additionally failed to clean image stage {}: {cleanup_error}",
            stage_dir.display()
        ),
    }
}

fn bmp_header(dib: &[u8]) -> anyhow::Result<[u8; 14]> {
    let dib_pixel_offset = super::dib_pixel_offset(dib)?;
    let pixel_offset = 14usize
        .checked_add(dib_pixel_offset)
        .ok_or_else(|| anyhow!("bitmap pixel offset overflow"))?;
    let file_size = 14usize
        .checked_add(dib.len())
        .ok_or_else(|| anyhow!("bitmap file size overflow"))?;
    let pixel_offset = u32::try_from(pixel_offset).context("bitmap pixel offset is too large")?;
    let file_size = u32::try_from(file_size).context("bitmap file is too large")?;

    let mut header = [0u8; 14];
    header[0..2].copy_from_slice(b"BM");
    header[2..6].copy_from_slice(&file_size.to_le_bytes());
    header[10..14].copy_from_slice(&pixel_offset.to_le_bytes());
    Ok(header)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn dib32(header_size: usize, bgra: [u8; 4]) -> Vec<u8> {
        let mut dib = vec![0u8; header_size];
        dib[0..4].copy_from_slice(&(header_size as u32).to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[20..24].copy_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&bgra);
        dib
    }

    fn equivalent_2x2_dib32() -> Vec<u8> {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-2i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[20..24].copy_from_slice(&16u32.to_le_bytes());
        dib.extend_from_slice(&[
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
        ]);
        dib
    }

    fn bottom_up_2x2_dib24() -> Vec<u8> {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[20..24].copy_from_slice(&16u32.to_le_bytes());
        dib.extend_from_slice(&[
            255, 0, 0, 255, 255, 255, 0xAA, 0xBB, 0, 0, 255, 0, 255, 0, 0xCC, 0xDD,
        ]);
        dib
    }

    fn truncated_decodable_header() -> Vec<u8> {
        let mut dib = dib32(40, [3, 2, 1, 255]);
        dib.truncate(40);
        dib
    }

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("super-clipboard-{label}-{}", Uuid::new_v4()))
    }

    #[test]
    fn hashes_a_one_by_one_dib_semantically() {
        let identity = image_identity_from_dib(&dib32(40, [3, 2, 1, 255])).expect("identity");
        assert_eq!(identity.width, 1);
        assert_eq!(identity.height, 1);
        assert_eq!(
            identity.content_hash,
            "26633a34f877ab76f1a9b09be0a021cfb7fe0ed963b5132cb9c3ab910dd499f6"
        );
    }

    #[test]
    fn ignores_trailing_global_size_padding() {
        let dib = dib32(40, [3, 2, 1, 255]);
        let mut padded = dib.clone();
        padded.extend_from_slice(&[0xAA; 64]);
        assert_eq!(
            image_identity_from_dib(&dib).expect("plain identity"),
            image_identity_from_dib(&padded).expect("padded identity")
        );
    }

    #[test]
    fn equivalent_info_and_v5_headers_have_the_same_identity() {
        assert_eq!(
            image_identity_from_dib(&dib32(40, [30, 20, 10, 255])).expect("info identity"),
            image_identity_from_dib(&dib32(124, [30, 20, 10, 255])).expect("v5 identity")
        );
    }

    #[test]
    fn handles_24_bit_bottom_up_rows_with_scanline_padding() {
        assert_eq!(
            image_identity_from_dib(&bottom_up_2x2_dib24()).expect("24-bit identity"),
            image_identity_from_dib(&equivalent_2x2_dib32()).expect("32-bit identity")
        );
    }

    #[test]
    fn different_pixels_have_different_identities() {
        assert_ne!(
            image_identity_from_dib(&dib32(40, [30, 20, 10, 255])).expect("first"),
            image_identity_from_dib(&dib32(40, [31, 20, 10, 255])).expect("second")
        );
    }

    #[test]
    fn rejects_invalid_dib() {
        assert!(image_identity_from_dib(b"not a dib").is_err());
    }

    #[test]
    fn canonical_paths_require_strict_lowercase_sha256() {
        let root = temp_dir("canonical");
        fs::create_dir_all(&root).expect("root");
        let hash = "26633a34f877ab76f1a9b09be0a021cfb7fe0ed963b5132cb9c3ab910dd499f6";

        assert_eq!(
            canonical_bmp_path(&root, hash).expect("bmp path"),
            root.canonicalize()
                .expect("canonical root")
                .join(format!("{hash}.bmp"))
        );
        assert_eq!(
            canonical_thumbnail_path(&root, hash).expect("thumbnail path"),
            root.canonicalize()
                .expect("canonical root")
                .join(format!("{hash}.thumb.png"))
        );
        for invalid in [
            "../escape",
            "26633A34F877AB76F1A9B09BE0A021CFB7FE0ED963B5132CB9C3AB910DD499F6",
            "abc",
            "26633a34f877ab76f1a9b09be0a021cfb7fe0ed963b5132cb9c3ab910dd499f60",
        ] {
            assert!(
                canonical_bmp_path(&root, invalid).is_err(),
                "accepted {invalid}"
            );
        }
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn stages_a_flushed_bmp_and_thumbnail() {
        let root = temp_dir("stage-image");
        let staged = stage_dib(&root, dib32(40, [30, 20, 10, 255])).expect("staged image");
        assert!(staged.bmp_path().is_file());
        assert!(staged.thumbnail_path().is_file());
        assert_eq!(
            fs::metadata(staged.bmp_path()).expect("bmp metadata").len(),
            staged.bmp_size()
        );
        assert_eq!(
            fs::metadata(staged.thumbnail_path())
                .expect("thumbnail metadata")
                .len(),
            staged.thumbnail_size()
        );
        assert_eq!(
            staged.bmp_path().file_name().and_then(|name| name.to_str()),
            Some(format!("{}.bmp", staged.content_hash()).as_str())
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn decode_failure_after_stage_creation_removes_uuid_stage() {
        let root = temp_dir("stage-decode-failure");
        fs::create_dir_all(&root).expect("stage root");

        assert!(stage_dib(&root, truncated_decodable_header()).is_err());
        assert!(fs::read_dir(&root).expect("read root").next().is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
