use super::{MAX_PREVIEW_BYTES, PreviewError, images};
use std::io::{Read, Seek, SeekFrom};

const HEAD_BYTES: u64 = 4 << 20;
const TAIL_BYTES: u64 = 1 << 20;

pub fn extract_gcode_thumbnail(mut reader: impl Read + Seek) -> Option<Vec<u8>> {
    let length = reader.seek(SeekFrom::End(0)).ok()?;
    reader.rewind().ok()?;
    let mut header = Vec::new();
    reader
        .by_ref()
        .take(HEAD_BYTES)
        .read_to_end(&mut header)
        .ok()?;
    if header.starts_with(b"GCDE") {
        reader.rewind().ok()?;
        return binary(reader).ok();
    }
    let mut best = text(&header);
    if length > HEAD_BYTES {
        reader
            .seek(SeekFrom::Start(length.saturating_sub(TAIL_BYTES)))
            .ok()?;
        let mut tail = Vec::new();
        reader.take(TAIL_BYTES).read_to_end(&mut tail).ok()?;
        choose(&mut best, text(&tail));
    }
    best
}

fn choose(best: &mut Option<Vec<u8>>, candidate: Option<Vec<u8>>) {
    if let Some(candidate) = candidate
        && best
            .as_ref()
            .is_none_or(|current| images::png_area(&candidate) > images::png_area(current))
    {
        *best = Some(candidate);
    }
}

fn text(bytes: &[u8]) -> Option<Vec<u8>> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut best = None;
    let mut block: Option<(&str, String)> = None;
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Some(comment) = line.trim().strip_prefix(';').map(str::trim) else {
            continue;
        };
        let mut words = comment.split_whitespace();
        let tag = words.next().unwrap_or_default();
        if matches!(tag, "thumbnail" | "thumbnail_JPG" | "thumbnail_QOI") {
            match words.next() {
                Some("begin") => block = Some((tag, String::new())),
                Some("end") => {
                    if let Some((begin_tag, data)) = block.take()
                        && begin_tag == tag
                        && let Ok(bytes) = STANDARD.decode(data)
                        && matches!(
                            (tag, image::guess_format(&bytes)),
                            ("thumbnail", Ok(image::ImageFormat::Png))
                                | ("thumbnail_JPG", Ok(image::ImageFormat::Jpeg))
                                | ("thumbnail_QOI", Ok(image::ImageFormat::Qoi))
                        )
                    {
                        choose(&mut best, images::normalize(&bytes).ok());
                    }
                }
                _ => block = None,
            }
        } else if let Some((_, data)) = block.as_mut() {
            if data.len() + comment.len() <= MAX_PREVIEW_BYTES * 4 / 3 + 4 {
                data.push_str(comment);
            } else {
                block = None;
            }
        }
    }
    best
}

fn binary(mut reader: impl Read + Seek) -> Result<Vec<u8>, PreviewError> {
    let mut header = [0; 10];
    reader
        .read_exact(&mut header)
        .map_err(|_| PreviewError::InvalidModel)?;
    if &header[..4] != b"GCDE" || u32::from_le_bytes(header[4..8].try_into().unwrap()) != 1 {
        return Err(PreviewError::Unsupported);
    }
    let checksum = match u16::from_le_bytes(header[8..10].try_into().unwrap()) {
        0 => 0,
        1 => 4,
        _ => return Err(PreviewError::Unsupported),
    };
    let mut best = None;
    let mut budget = 32 << 20;
    for _ in 0..1024 {
        let mut block = [0; 8];
        if reader.read_exact(&mut block).is_err() {
            break;
        }
        let kind = u16::from_le_bytes(block[..2].try_into().unwrap());
        if kind == 1 {
            break;
        } // Never scan or decompress toolpaths.
        if kind > 5 {
            return Err(PreviewError::Unsupported);
        }
        let compression = u16::from_le_bytes(block[2..4].try_into().unwrap());
        let uncompressed = u32::from_le_bytes(block[4..8].try_into().unwrap()) as usize;
        let length = if compression == 0 {
            uncompressed
        } else {
            let mut size = [0; 4];
            reader
                .read_exact(&mut size)
                .map_err(|_| PreviewError::InvalidModel)?;
            u32::from_le_bytes(size) as usize
        };
        let params_len = if kind == 5 { 6 } else { 2 };
        let mut params = [0; 6];
        reader
            .read_exact(&mut params[..params_len])
            .map_err(|_| PreviewError::InvalidModel)?;
        if kind != 5 || uncompressed > MAX_PREVIEW_BYTES || length > budget {
            reader
                .seek(SeekFrom::Current((length + checksum) as i64))
                .map_err(|_| PreviewError::InvalidModel)?;
            continue;
        }
        budget -= length;
        let mut data = vec![0; length];
        reader
            .read_exact(&mut data)
            .map_err(|_| PreviewError::InvalidModel)?;
        reader
            .seek(SeekFrom::Current(checksum as i64))
            .map_err(|_| PreviewError::InvalidModel)?;
        let data = match compression {
            0 => data,
            1 => {
                let mut output = Vec::new();
                flate2::read::ZlibDecoder::new(data.as_slice())
                    .take(MAX_PREVIEW_BYTES as u64 + 1)
                    .read_to_end(&mut output)
                    .map_err(|_| PreviewError::InvalidModel)?;
                if output.len() != uncompressed {
                    return Err(PreviewError::InvalidModel);
                }
                output
            }
            _ => continue, // Heatshrink is not needed for ordinary BGCode thumbnails.
        };
        choose(&mut best, images::normalize(&data).ok());
    }
    best.ok_or(PreviewError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn accepts_jpeg_and_qoi_and_selects_by_decoded_area() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let mut text = String::new();
        for (tag, format, width) in [
            ("thumbnail_JPG", image::ImageFormat::Jpeg, 3),
            ("thumbnail_QOI", image::ImageFormat::Qoi, 8),
        ] {
            let mut encoded = Cursor::new(Vec::new());
            image::RgbImage::from_pixel(width, 2, image::Rgb([20, 80, 120]))
                .write_to(&mut encoded, format)
                .unwrap();
            text.push_str(&format!(
                "; {tag} begin {width}x2 0\n; {}\n; {tag} end\n",
                STANDARD.encode(encoded.get_ref())
            ));
        }
        let png = extract_gcode_thumbnail(Cursor::new(text)).unwrap();
        assert_eq!(images::png_area(&png), 16);
    }

    #[test]
    fn reads_footer_without_reading_a_large_toolpath() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let png = super::super::encode_png(&[0; 3], (1, 1)).unwrap();
        let mut data = vec![b' '; HEAD_BYTES as usize + 100];
        data.extend_from_slice(
            format!(
                "\n; thumbnail begin 1x1 0\n; {}\n; thumbnail end\n",
                STANDARD.encode(&png)
            )
            .as_bytes(),
        );
        assert_eq!(extract_gcode_thumbnail(Cursor::new(data)), Some(png));
    }

    #[test]
    fn reads_compressed_bgcode_thumbnail_and_rejects_bad_version() {
        let png = super::super::encode_png(&[0; 3], (1, 1)).unwrap();
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&png).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut data = b"GCDE\x01\0\0\0\0\0".to_vec();
        data.extend_from_slice(&5u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&(png.len() as u32).to_le_bytes());
        data.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        data.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
        data.extend_from_slice(&compressed);
        assert_eq!(extract_gcode_thumbnail(Cursor::new(&data)), Some(png));
        data[4] = 2;
        assert!(extract_gcode_thumbnail(Cursor::new(data)).is_none());
    }
}
