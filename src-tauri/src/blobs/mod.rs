use std::fs;
use std::path::{Path, PathBuf};

use ::image::ImageReader;
use uuid::Uuid;

pub mod image;
pub mod store;

pub fn ensure_blob_dir(app_data: &Path) -> anyhow::Result<PathBuf> {
    let dir = app_data.join("blobs");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn build_blob_path(blob_dir: &Path, extension: &str) -> PathBuf {
    let safe_extension = extension.trim_start_matches('.').trim();
    let extension = if safe_extension.is_empty() {
        "bin"
    } else {
        safe_extension
    };
    blob_dir.join(format!("{}.{}", Uuid::new_v4(), extension))
}

pub fn remove_blob_if_exists(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn clear_blob_dir(blob_dir: &Path) -> anyhow::Result<()> {
    if !blob_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(blob_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            remove_blob_if_exists(&path)?;
        }
    }

    Ok(())
}

pub fn write_dib_as_bmp(blob_dir: &Path, dib_bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let path = build_blob_path(blob_dir, "bmp");
    let bmp_bytes = bmp_file_from_dib(dib_bytes)?;
    write_file_with_cleanup(&path, |target| {
        fs::write(target, &bmp_bytes)?;
        Ok(())
    })?;
    if let Err(e) = create_thumbnail(&path) {
        crate::diagnostics::warn(format!(
            "blobs: thumbnail creation failed for {}: {}",
            path.display(),
            e
        ));
    }
    Ok(path)
}

fn write_file_with_cleanup<F>(target: &Path, writer: F) -> anyhow::Result<()>
where
    F: FnOnce(&Path) -> anyhow::Result<()>,
{
    if let Err(error) = writer(target) {
        let _ = remove_blob_if_exists(target);
        return Err(error);
    }
    Ok(())
}

pub fn read_dib_from_bmp_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    bmp_file_to_dib(&bytes)
}

pub fn thumbnail_path_for(blob_path: &Path) -> PathBuf {
    let stem = blob_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("thumbnail");
    blob_path.with_file_name(format!("{stem}.thumb.png"))
}

pub fn create_thumbnail(blob_path: &Path) -> anyhow::Result<PathBuf> {
    let thumbnail_path = thumbnail_path_for(blob_path);
    let image = ImageReader::open(blob_path)?
        .with_guessed_format()?
        .decode()?;
    image.thumbnail(320, 320).save(&thumbnail_path)?;
    Ok(thumbnail_path)
}

fn bmp_file_from_dib(dib_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let dib_pixel_offset = dib_pixel_offset(dib_bytes)?;
    if dib_pixel_offset > dib_bytes.len() {
        return Err(anyhow::anyhow!("DIB pixel offset is out of bounds"));
    }

    let pixel_offset = 14usize
        .checked_add(dib_pixel_offset)
        .ok_or_else(|| anyhow::anyhow!("bitmap pixel offset overflow"))?;
    let file_size = 14usize
        .checked_add(dib_bytes.len())
        .ok_or_else(|| anyhow::anyhow!("bitmap file size overflow"))?;
    if pixel_offset > u32::MAX as usize || file_size > u32::MAX as usize {
        return Err(anyhow::anyhow!("bitmap file is too large"));
    }

    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib_bytes);
    Ok(bmp)
}

fn bmp_file_to_dib(bmp_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if bmp_bytes.len() < 14 || &bmp_bytes[..2] != b"BM" {
        return Err(anyhow::anyhow!("image blob is not a BMP file"));
    }
    Ok(bmp_bytes[14..].to_vec())
}

fn dib_pixel_offset(dib_bytes: &[u8]) -> anyhow::Result<usize> {
    if dib_bytes.len() < 16 {
        return Err(anyhow::anyhow!("DIB payload is too small"));
    }

    let header_size = read_u32_le(dib_bytes, 0)? as usize;
    if header_size < 12 || header_size > dib_bytes.len() {
        return Err(anyhow::anyhow!("unsupported DIB header size"));
    }

    if header_size == 12 {
        let bit_count = read_u16_le(dib_bytes, 10)?;
        let palette_entries = palette_entries_for_bit_count(bit_count);
        return header_size
            .checked_add(palette_entries.saturating_mul(3))
            .ok_or_else(|| anyhow::anyhow!("DIB pixel offset overflow"));
    }

    let bit_count = read_u16_le(dib_bytes, 14)?;
    let compression = read_u32_le(dib_bytes, 16)?;
    let colors_used = if dib_bytes.len() >= 36 {
        read_u32_le(dib_bytes, 32)? as usize
    } else {
        0
    };
    let palette_entries = if colors_used > 0 {
        colors_used
    } else {
        palette_entries_for_bit_count(bit_count)
    };
    let mask_bytes = match (header_size, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };

    header_size
        .checked_add(mask_bytes)
        .and_then(|value| value.checked_add(palette_entries.saturating_mul(4)))
        .ok_or_else(|| anyhow::anyhow!("DIB pixel offset overflow"))
}

fn palette_entries_for_bit_count(bit_count: u16) -> usize {
    if bit_count <= 8 {
        1usize << usize::from(bit_count)
    } else {
        0
    }
}

fn read_u16_le(bytes: &[u8], offset: usize) -> anyhow::Result<u16> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| anyhow::anyhow!("DIB u16 field is out of bounds"))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("DIB u32 field is out of bounds"))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod write_cleanup_tests {
    use super::write_file_with_cleanup;
    use anyhow::anyhow;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn partial_file_is_removed_when_writer_fails() {
        let dir = std::env::temp_dir().join(format!("super-clipboard-write-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("partial.bmp");

        let error = write_file_with_cleanup(&target, |path| {
            fs::write(path, b"partial")?;
            Err(anyhow!("simulated write failure"))
        })
        .expect_err("writer should fail");

        assert_eq!(error.to_string(), "simulated write failure");
        assert!(!target.exists());
        fs::remove_dir_all(dir).expect("clean temp dir");
    }

    #[test]
    fn successful_file_is_preserved() {
        let dir = std::env::temp_dir().join(format!("super-clipboard-write-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("complete.bmp");

        write_file_with_cleanup(&target, |path| {
            fs::write(path, b"complete")?;
            Ok(())
        })
        .expect("writer succeeds");

        assert_eq!(fs::read(&target).expect("read target"), b"complete");
        fs::remove_dir_all(dir).expect("clean temp dir");
    }

    #[test]
    fn cleanup_failure_preserves_the_writer_error() {
        let target = std::env::temp_dir().join(format!("super-clipboard-write-{}", Uuid::new_v4()));
        fs::create_dir_all(&target).expect("temp dir target");

        let error =
            write_file_with_cleanup(&target, |_path| Err(anyhow!("original write failure")))
                .expect_err("writer should fail");

        assert_eq!(error.to_string(), "original write failure");
        assert!(target.exists());
        fs::remove_dir_all(target).expect("clean temp dir");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::image::{ImageBuffer, Rgba};

    #[test]
    fn build_blob_path_keeps_requested_extension() {
        let path = build_blob_path(Path::new("root"), ".png");
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("png")
        );
    }

    #[test]
    fn thumbnail_path_uses_png_extension() {
        let path = thumbnail_path_for(Path::new("root/item.jpg"));
        assert_eq!(
            path.file_name().and_then(|value| value.to_str()),
            Some("item.thumb.png")
        );
    }

    #[test]
    fn create_thumbnail_writes_thumbnail_file() {
        let dir = std::env::temp_dir().join(format!("super-clipboard-blob-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");
        let image_path = dir.join("source.png");
        let image = ImageBuffer::from_pixel(8, 8, Rgba([80u8, 120u8, 200u8, 255u8]));
        image.save(&image_path).expect("source image");

        let thumbnail_path = create_thumbnail(&image_path).expect("thumbnail");

        assert!(thumbnail_path.exists());
    }

    #[test]
    fn write_dib_as_bmp_wraps_dib_and_creates_thumbnail() {
        let dir = std::env::temp_dir().join(format!("super-clipboard-blob-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");

        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&(-1i32).to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&[0, 0, 255, 255]);

        let path = write_dib_as_bmp(&dir, &dib).expect("bmp blob");
        let bytes = fs::read(&path).expect("bmp file");

        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("bmp")
        );
        assert_eq!(&bytes[..2], b"BM");
        assert!(thumbnail_path_for(&path).exists());
    }

    #[test]
    fn read_dib_from_bmp_file_returns_original_dib_payload() {
        let dir = std::env::temp_dir().join(format!("super-clipboard-blob-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("temp dir");

        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&(-1i32).to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&[0, 0, 255, 255]);

        let path = write_dib_as_bmp(&dir, &dib).expect("bmp blob");
        let restored = read_dib_from_bmp_file(&path).expect("dib payload");

        assert_eq!(restored, dib);
    }
}
