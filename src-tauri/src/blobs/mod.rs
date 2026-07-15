use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::storage::capacity::MAX_IMAGE_ALLOCATION;

pub mod image;
pub mod store;

const BMP_FILE_HEADER_LEN: u64 = 14;

pub fn ensure_blob_dir(app_data: &Path) -> anyhow::Result<PathBuf> {
    let dir = app_data.join("blobs");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn read_dib_from_bmp_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut file = File::open(path)
        .with_context(|| format!("failed to open BMP image blob: {}", path.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("failed to read BMP image blob metadata: {}", path.display()))?
        .len();
    read_dib_from_bmp(&mut file, file_len)
        .with_context(|| format!("failed to read BMP image blob: {}", path.display()))
}

fn read_dib_from_bmp(reader: &mut impl Read, file_len: u64) -> anyhow::Result<Vec<u8>> {
    validate_bmp_file_header(reader, file_len)?;

    let payload_len = file_len
        .checked_sub(BMP_FILE_HEADER_LEN)
        .ok_or_else(|| anyhow::anyhow!("BMP payload length underflow"))?;
    anyhow::ensure!(
        payload_len <= MAX_IMAGE_ALLOCATION,
        "DIB payload exceeds the 100 MiB image allocation limit"
    );
    let payload_len = usize::try_from(payload_len)
        .map_err(|_| anyhow::anyhow!("DIB payload length does not fit in memory"))?;

    let mut dib = Vec::new();
    dib.try_reserve_exact(payload_len)
        .context("failed to allocate DIB payload")?;
    dib.resize(payload_len, 0);
    reader
        .read_exact(&mut dib)
        .context("failed to read DIB payload")?;
    Ok(dib)
}

pub(crate) fn validate_bmp_file_header(
    reader: &mut impl Read,
    file_len: u64,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        file_len >= BMP_FILE_HEADER_LEN,
        "BMP file header is truncated"
    );
    anyhow::ensure!(
        file_len <= MAX_IMAGE_ALLOCATION,
        "BMP file exceeds the 100 MiB image allocation limit"
    );

    let mut header = [0_u8; BMP_FILE_HEADER_LEN as usize];
    reader
        .read_exact(&mut header)
        .context("failed to read BMP file header")?;
    anyhow::ensure!(&header[..2] == b"BM", "image blob is not a BMP file");

    let declared_file_len = u32::from_le_bytes(header[2..6].try_into().expect("fixed BMP header"));
    anyhow::ensure!(
        u64::from(declared_file_len) == file_len,
        "BMP declared file size does not match blob length"
    );
    let pixel_offset = u32::from_le_bytes(header[10..14].try_into().expect("fixed BMP header"));
    anyhow::ensure!(
        u64::from(pixel_offset) >= BMP_FILE_HEADER_LEN && u64::from(pixel_offset) <= file_len,
        "BMP pixel offset is out of bounds"
    );
    Ok(())
}

pub fn thumbnail_path_for(blob_path: &Path) -> PathBuf {
    let stem = blob_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("thumbnail");
    blob_path.with_file_name(format!("{stem}.thumb.png"))
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
mod tests {
    use std::cell::RefCell;
    use std::io::{self, Read};
    use std::rc::Rc;

    use super::*;
    use uuid::Uuid;

    struct RecordingReader {
        bytes: Vec<u8>,
        position: usize,
        requests: Rc<RefCell<Vec<usize>>>,
    }

    impl Read for RecordingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.requests.borrow_mut().push(buffer.len());
            let remaining = &self.bytes[self.position..];
            let read_len = remaining.len().min(buffer.len());
            buffer[..read_len].copy_from_slice(&remaining[..read_len]);
            self.position += read_len;
            Ok(read_len)
        }
    }

    struct PanicReader;

    impl Read for PanicReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            panic!("oversized BMP must be rejected before reading or allocating")
        }
    }

    fn test_bmp(dib: &[u8]) -> Vec<u8> {
        let file_len = 14_u32 + u32::try_from(dib.len()).expect("test DIB length");
        let mut bmp = Vec::with_capacity(file_len as usize);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&file_len.to_le_bytes());
        bmp.extend_from_slice(&0_u32.to_le_bytes());
        bmp.extend_from_slice(&14_u32.to_le_bytes());
        bmp.extend_from_slice(dib);
        bmp
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

        let path = dir.join("image.bmp");
        fs::write(&path, bmp_file_from_dib(&dib).expect("bmp bytes")).expect("bmp blob");
        let restored = read_dib_from_bmp_file(&path).expect("dib payload");

        assert_eq!(restored, dib);
    }

    #[test]
    fn dib_from_bmp_reader_returns_every_byte_after_the_file_header() {
        let dib = [40, 0, 0, 0, 0xaa, 0xbb, 0xcc, 0xdd];
        let mut bmp = test_bmp(&dib);
        bmp[10..14].copy_from_slice(&18_u32.to_le_bytes());

        let restored =
            read_dib_from_bmp(&mut bmp.as_slice(), bmp.len() as u64).expect("valid BMP payload");

        assert_eq!(restored, dib);
    }

    #[test]
    fn dib_from_bmp_reader_rejects_truncated_or_inconsistent_headers() {
        let valid = test_bmp(&[40, 0, 0, 0]);
        let cases = [
            (b"not a bitmap".to_vec(), 12_u64),
            (valid[..10].to_vec(), valid.len() as u64),
            (valid[..valid.len() - 1].to_vec(), valid.len() as u64),
            (
                {
                    let mut bytes = valid.clone();
                    bytes[0..2].copy_from_slice(b"ZZ");
                    bytes
                },
                valid.len() as u64,
            ),
            (
                {
                    let mut bytes = valid.clone();
                    bytes[2..6].copy_from_slice(&((valid.len() as u32) - 1).to_le_bytes());
                    bytes
                },
                valid.len() as u64,
            ),
            (
                {
                    let mut bytes = valid.clone();
                    bytes[10..14].copy_from_slice(&13_u32.to_le_bytes());
                    bytes
                },
                valid.len() as u64,
            ),
            (
                {
                    let mut bytes = valid.clone();
                    bytes[10..14].copy_from_slice(&((valid.len() as u32) + 1).to_le_bytes());
                    bytes
                },
                valid.len() as u64,
            ),
        ];

        for (bytes, declared_len) in cases {
            assert!(
                read_dib_from_bmp(&mut bytes.as_slice(), declared_len).is_err(),
                "invalid BMP header was accepted: {bytes:?}"
            );
        }
    }

    #[test]
    fn dib_from_bmp_reader_rejects_oversized_lengths_before_reading() {
        for file_len in [MAX_IMAGE_ALLOCATION + 1, u64::MAX] {
            let error =
                read_dib_from_bmp(&mut PanicReader, file_len).expect_err("oversized BMP must fail");
            assert!(error.to_string().contains("100 MiB"), "{error:#}");
        }
    }

    #[test]
    fn dib_from_bmp_reader_reads_directly_into_one_payload_buffer() {
        let dib = vec![0x5a; 32];
        let bmp = test_bmp(&dib);
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut reader = RecordingReader {
            bytes: bmp.clone(),
            position: 0,
            requests: Rc::clone(&requests),
        };

        let restored = read_dib_from_bmp(&mut reader, bmp.len() as u64).expect("valid BMP payload");

        assert_eq!(restored, dib);
        assert_eq!(&*requests.borrow(), &[14, dib.len()]);
    }
}
