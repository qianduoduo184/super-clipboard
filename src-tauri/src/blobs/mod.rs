use std::fs;
use std::path::{Path, PathBuf};

use image::ImageReader;
use uuid::Uuid;

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
            fs::remove_file(path)?;
        }
    }

    Ok(())
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
    let image = ImageReader::open(blob_path)?.with_guessed_format()?.decode()?;
    image.thumbnail(320, 320).save(&thumbnail_path)?;
    Ok(thumbnail_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_blob_path_keeps_requested_extension() {
        let path = build_blob_path(Path::new("root"), ".png");
        assert_eq!(path.extension().and_then(|value| value.to_str()), Some("png"));
    }

    #[test]
    fn thumbnail_path_uses_png_extension() {
        let path = thumbnail_path_for(Path::new("root/item.jpg"));
        assert_eq!(path.file_name().and_then(|value| value.to_str()), Some("item.thumb.png"));
    }
}
