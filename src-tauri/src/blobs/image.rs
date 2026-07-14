use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
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
    pub content_hash: String,
    pub bmp_path: PathBuf,
    pub thumbnail_path: PathBuf,
    pub bmp_size: u64,
    pub thumbnail_size: u64,
}

pub fn image_identity_from_dib(dib: &[u8]) -> anyhow::Result<ImageIdentity> {
    let bmp = super::bmp_file_from_dib(dib).context("wrap DIB as BMP")?;
    let decoded = ::image::load_from_memory_with_format(&bmp, ::image::ImageFormat::Bmp)
        .context("decode DIB pixels")?;
    let rgba = decoded.into_rgba8();
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

    Ok(ImageIdentity {
        content_hash,
        width,
        height,
    })
}

pub fn canonical_bmp_path(blob_dir: &Path, content_hash: &str) -> PathBuf {
    blob_dir.join(format!("{content_hash}.bmp"))
}

pub fn canonical_thumbnail_path(blob_dir: &Path, content_hash: &str) -> PathBuf {
    blob_dir.join(format!("{content_hash}.thumb.png"))
}

pub fn stage_dib(stage_root: &Path, dib: Vec<u8>) -> anyhow::Result<StagedImage> {
    let identity = image_identity_from_dib(&dib)?;
    fs::create_dir_all(stage_root)
        .with_context(|| format!("create stage root {}", stage_root.display()))?;
    let stage_dir = stage_root.join(Uuid::new_v4().to_string());
    fs::create_dir(&stage_dir)
        .with_context(|| format!("create image stage {}", stage_dir.display()))?;

    match stage_dib_in(&stage_dir, dib, identity) {
        Ok(staged) => Ok(staged),
        Err(stage_error) => {
            if let Err(cleanup_error) = fs::remove_dir_all(&stage_dir) {
                return Err(cleanup_error).with_context(|| {
                    format!(
                        "clean failed image stage {} after: {stage_error:#}",
                        stage_dir.display()
                    )
                });
            }
            Err(stage_error)
        }
    }
}

fn stage_dib_in(
    stage_dir: &Path,
    dib: Vec<u8>,
    identity: ImageIdentity,
) -> anyhow::Result<StagedImage> {
    let bmp_path = canonical_bmp_path(stage_dir, &identity.content_hash);
    let thumbnail_path = canonical_thumbnail_path(stage_dir, &identity.content_hash);
    let header = bmp_header(&dib)?;

    let mut bmp_file = File::create(&bmp_path)
        .with_context(|| format!("create staged BMP {}", bmp_path.display()))?;
    bmp_file.write_all(&header)?;
    bmp_file.write_all(&dib)?;
    bmp_file.flush()?;
    bmp_file.sync_all()?;
    let bmp_size = bmp_file.metadata()?.len();
    drop(bmp_file);
    drop(dib);

    let decoded = ::image::ImageReader::open(&bmp_path)?
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("decode staged BMP {}", bmp_path.display()))?;
    let thumbnail = decoded.thumbnail(320, 320);
    let thumbnail_file = File::create(&thumbnail_path)
        .with_context(|| format!("create thumbnail {}", thumbnail_path.display()))?;
    let mut thumbnail_writer = BufWriter::new(thumbnail_file);
    thumbnail.write_to(&mut thumbnail_writer, ::image::ImageFormat::Png)?;
    thumbnail_writer.flush()?;
    thumbnail_writer.get_ref().sync_all()?;
    let thumbnail_size = thumbnail_writer.get_ref().metadata()?.len();

    Ok(StagedImage {
        content_hash: identity.content_hash,
        bmp_path,
        thumbnail_path,
        bmp_size,
        thumbnail_size,
    })
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
    use std::path::Path;
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

    fn temp_dir(label: &str) -> std::path::PathBuf {
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
    fn canonical_paths_use_the_content_hash() {
        let root = Path::new("blobs");
        assert_eq!(canonical_bmp_path(root, "abc"), root.join("abc.bmp"));
        assert_eq!(
            canonical_thumbnail_path(root, "abc"),
            root.join("abc.thumb.png")
        );
    }

    #[test]
    fn stages_a_flushed_bmp_and_thumbnail() {
        let root = temp_dir("stage-image");
        let staged = stage_dib(&root, dib32(40, [30, 20, 10, 255])).expect("staged image");
        assert!(staged.bmp_path.is_file());
        assert!(staged.thumbnail_path.is_file());
        assert_eq!(
            fs::metadata(&staged.bmp_path).expect("bmp metadata").len(),
            staged.bmp_size
        );
        assert_eq!(
            fs::metadata(&staged.thumbnail_path)
                .expect("thumbnail metadata")
                .len(),
            staged.thumbnail_size
        );
        assert_eq!(
            staged.bmp_path.file_name().and_then(|name| name.to_str()),
            Some(format!("{}.bmp", staged.content_hash).as_str())
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn failed_stage_removes_its_stage_directory() {
        let root = temp_dir("stage-failure");
        assert!(stage_dib(&root, b"invalid".to_vec()).is_err());
        assert!(!root.exists() || fs::read_dir(&root).expect("read root").next().is_none());
        let _ = fs::remove_dir_all(root);
    }
}
