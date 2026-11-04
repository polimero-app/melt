//! Stored application previews. Format rules adapted from ModelThumbs; see
//! THIRD_PARTY_NOTICES.md. No application, macro, script or geometry kernel runs.
use super::{MAX_PREVIEW_BYTES, PreviewError, images, read_bounded};
use std::io::{Cursor, Read, Seek};
use std::path::Path;

const PREFIX_BYTES: usize = 64 << 20;
const CFB_MAGIC: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";

fn u16_at(data: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(p..p + 2)?.try_into().ok()?))
}
fn u32_at(data: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?))
}

fn png_extent(data: &[u8], start: usize) -> Option<usize> {
    if data.get(start..start + 8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut p = start + 8;
    while p.checked_sub(start)? <= MAX_PREVIEW_BYTES {
        let header = data.get(p..p + 8)?;
        let length = u32::from_be_bytes(header[..4].try_into().ok()?) as usize;
        p = p.checked_add(length)?.checked_add(12)?;
        if p > data.len() || p - start > MAX_PREVIEW_BYTES {
            return None;
        }
        if &header[4..] == b"IEND" {
            return Some(p);
        }
    }
    None
}

/// Complete chunk-delimited PNGs only; never copy the rest of a model file.
fn embedded_png(data: &[u8], first: bool) -> Option<Vec<u8>> {
    let mut candidates = Vec::new();
    let mut p = 0;
    let mut attempts = 0;
    while p + 8 <= data.len() && attempts < 256 {
        if data[p..].starts_with(b"\x89PNG\r\n\x1a\n") {
            attempts += 1;
            if let Some(end) = png_extent(data, p) {
                let png = &data[p..end];
                if super::validate_png(png) {
                    if first {
                        if let Ok(png) = images::normalize(png) {
                            return Some(png);
                        }
                    } else {
                        candidates.push((images::png_area(png), p, end));
                    }
                }
                p = end;
                continue;
            }
        }
        p += 1;
    }
    candidates.sort_by_key(|(area, _, _)| std::cmp::Reverse(*area));
    for (_, start, end) in candidates {
        if let Ok(png) = images::normalize(&data[start..end]) {
            return Some(png);
        }
    }
    None
}

/// Wrap a Windows DIB in a BMP file header and delegate pixels/palettes to image.
fn dib(data: &[u8], start: usize) -> Option<Vec<u8>> {
    let data = data.get(start..)?;
    let header = u32_at(data, 0)? as usize;
    let width = u32_at(data, 4)? as i32;
    let height = u32_at(data, 8)? as i32;
    let bpp = u16_at(data, 14)?;
    let compression = u32_at(data, 16)?;
    if !(40..=124).contains(&header)
        || width <= 0
        || height == 0
        || width > 4096
        || height.unsigned_abs() > 4096
        || u16_at(data, 12)? != 1
        || !matches!(bpp, 8 | 24 | 32)
        || !matches!(compression, 0 | 3)
    {
        return None;
    }
    let palette = if bpp == 8 {
        let used = u32_at(data, 32)? as usize;
        if used > 256 {
            return None;
        }
        if used == 0 { 256 } else { used }
    } else {
        0
    };
    let masks = if compression == 3 && header == 40 {
        12
    } else {
        0
    };
    let pixels = header + masks + palette * 4;
    let stride = (width as usize * bpp as usize).div_ceil(32) * 4;
    let length = pixels + stride * height.unsigned_abs() as usize;
    if length + 14 > MAX_PREVIEW_BYTES {
        return None;
    }
    let data = data.get(..length)?;
    let mut bmp = b"BM".to_vec();
    bmp.extend_from_slice(&((length + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((pixels + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(data);
    images::normalize(&bmp).ok()
}

fn image_payload(data: &[u8]) -> Option<Vec<u8>> {
    if (data.starts_with(b"\x89PNG") || data.starts_with(b"\xff\xd8") || data.starts_with(b"BM"))
        && let Ok(png) = images::normalize(data)
    {
        return Some(png);
    }
    embedded_png(data, false)
        .or_else(|| dib(data, 0))
        .or_else(|| dib(data, 4))
}

fn summary_thumbnail(data: &[u8]) -> Option<Vec<u8>> {
    let section = u32_at(data, 44)? as usize;
    let count = u32_at(data, section + 4)? as usize;
    if count > 1000 {
        return None;
    }
    for i in 0..count {
        let record = section + 8 + i * 8;
        if u32_at(data, record)? != 0x11 {
            continue;
        }
        let p = section.checked_add(u32_at(data, record + 4)? as usize)?;
        if u32_at(data, p)? != 0x47 {
            return None;
        }
        let length = u32_at(data, p + 4)? as usize;
        let format = u32_at(data, p + 8)?;
        if format == u32::MAX
            && u32_at(data, p + 12)? == 8
            && let Some(png) = dib(data, p + 16)
        {
            return Some(png);
        }
        return image_payload(data.get(p + 12..p.checked_add(8)?.checked_add(length)?)?);
    }
    None
}

fn compound(reader: impl Read + Seek) -> Result<Vec<u8>, PreviewError> {
    let mut cfb = cfb::CompoundFile::open(reader).map_err(|_| PreviewError::InvalidModel)?;
    let mut entries = cfb
        .walk()
        .filter(|entry| entry.is_stream())
        .take(100_001)
        .map(|entry| {
            let name = entry.path().to_string_lossy().to_ascii_lowercase();
            let rank = if name.contains("preview") || name.contains("thumb") {
                0
            } else if name.contains("summaryinformation") {
                1
            } else {
                2
            };
            (rank, entry.len(), entry.path().to_owned(), name)
        })
        .collect::<Vec<_>>();
    if entries.len() > 100_000 {
        return Err(PreviewError::TooLarge);
    }
    entries.sort_by_key(|(rank, _, _, _)| *rank);
    let mut budget = PREFIX_BYTES;
    for (_, len, path, name) in entries {
        if len > MAX_PREVIEW_BYTES.min(budget) as u64 {
            continue;
        }
        let stream = cfb
            .open_stream(path)
            .map_err(|_| PreviewError::InvalidModel)?;
        let data = read_bounded(stream, MAX_PREVIEW_BYTES.min(budget))?;
        budget -= data.len();
        if name.contains("summaryinformation")
            && let Some(png) = summary_thumbnail(&data)
        {
            return Ok(png);
        }
        if let Some(png) = image_payload(&data) {
            return Ok(png);
        }
    }
    Err(PreviewError::Unsupported)
}

fn solidworks(data: &[u8]) -> Option<Vec<u8>> {
    let marker = [0x14, 0, 6, 0, 8, 0];
    let mut blocks = Vec::new();
    for (p, window) in data.windows(marker.len()).enumerate() {
        if window != marker {
            continue;
        }
        if blocks.len() >= 4096 {
            break;
        }
        let Some(compressed) = u32_at(data, p + 14).map(|n| n as usize) else {
            continue;
        };
        let Some(uncompressed) = u32_at(data, p + 18).map(|n| n as usize) else {
            continue;
        };
        let Some(name_len) = u32_at(data, p + 22).map(|n| n as usize) else {
            continue;
        };
        if name_len > 1024 || uncompressed > MAX_PREVIEW_BYTES {
            continue;
        }
        let start = p + 26 + name_len;
        let Some(payload) = data.get(start..start + compressed) else {
            continue;
        };
        let Some(name) = data.get(p + 26..start) else {
            continue;
        };
        let name = name
            .iter()
            .map(|b| b.rotate_left(4) as char)
            .collect::<String>();
        blocks.push((
            !name.to_ascii_lowercase().contains("preview"),
            uncompressed,
            payload,
        ));
    }
    blocks.sort_by_key(|(unnamed, _, _)| *unnamed);
    let mut budget = PREFIX_BYTES;
    for (_, uncompressed, payload) in blocks {
        if uncompressed > budget {
            continue;
        }
        budget -= uncompressed;
        let Ok(data) = read_bounded(flate2::read::DeflateDecoder::new(payload), uncompressed)
        else {
            continue;
        };
        if data.len() == uncompressed
            && let Some(png) = image_payload(&data)
        {
            return Some(png);
        }
    }
    None
}

fn sketchup(data: &[u8]) -> Result<Vec<u8>, PreviewError> {
    let header = &data[..data.len().min(4 << 20)];
    if let Some(p) = header.windows(4).position(|bytes| bytes == b"CDib")
        && let Some(png) = embedded_png(&data[p + 4..data.len().min(p + 4 + (8 << 20))], true)
    {
        return Ok(png);
    }
    embedded_png(data, true).ok_or(PreviewError::Unsupported)
}

fn rhino(data: &[u8]) -> Result<Vec<u8>, PreviewError> {
    if !data.starts_with(b"3D Geometry File Format") {
        return Err(PreviewError::InvalidModel);
    }
    let mut budget = PREFIX_BYTES;
    for code in [0x20008025u32, 0x20008023] {
        let mut found = 0;
        for (p, bytes) in data.windows(4).enumerate() {
            if bytes != code.to_le_bytes() {
                continue;
            }
            found += 1;
            if found > 256 {
                break;
            }
            for h in [p + 8, p + 12] {
                if u32_at(data, h) != Some(40) {
                    continue;
                }
                if code == 0x20008023 {
                    if let Some(png) = dib(data, h) {
                        return Ok(png);
                    }
                    continue;
                }
                let Some(width) = u32_at(data, h + 4).filter(|w| *w > 0 && *w <= 4096) else {
                    continue;
                };
                let Some(height) = u32_at(data, h + 8)
                    .map(|v| (v as i32).unsigned_abs())
                    .filter(|h| *h > 0 && *h <= 4096)
                else {
                    continue;
                };
                let expected = (width as usize * 4 + 4) * height as usize + 1024;
                if expected > MAX_PREVIEW_BYTES {
                    continue;
                }
                for q in h + 40..data.len().saturating_sub(1).min(h + 120) {
                    if data[q] != 0x78
                        || !u16::from_be_bytes([data[q], data[q + 1]]).is_multiple_of(31)
                    {
                        continue;
                    }
                    if expected > budget {
                        continue;
                    }
                    budget -= expected;
                    if let Ok(raw) =
                        read_bounded(flate2::read::ZlibDecoder::new(&data[q..]), expected)
                    {
                        let mut rebuilt = data[h..h + 40].to_vec();
                        rebuilt.extend(raw);
                        if let Some(png) = dib(&rebuilt, 0) {
                            return Ok(png);
                        }
                    }
                }
            }
        }
    }
    Err(PreviewError::Unsupported)
}

fn blender(input: &[u8]) -> Result<Vec<u8>, PreviewError> {
    let decoded;
    let data = if input.starts_with(&[0x1f, 0x8b]) {
        let mut prefix = Vec::new();
        flate2::read::GzDecoder::new(input)
            .take(PREFIX_BYTES as u64)
            .read_to_end(&mut prefix)
            .map_err(|_| PreviewError::InvalidModel)?;
        decoded = prefix;
        decoded.as_slice()
    } else if input.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Err(PreviewError::Unsupported);
    } else {
        input
    };
    if !data.starts_with(b"BLENDER") || data.len() < 12 {
        return Err(PreviewError::InvalidModel);
    }
    let classic = matches!(data[7], b'_' | b'-');
    let (mut p, wide, big, pointer) = if classic {
        (
            12,
            false,
            data[8] == b'V',
            if data[7] == b'_' { 4 } else { 8 },
        )
    } else {
        let size = std::str::from_utf8(&data[7..9])
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or(PreviewError::InvalidModel)?;
        if !(17..=64).contains(&size) || data.len() < size {
            return Err(PreviewError::InvalidModel);
        }
        (size, true, data[12] == b'V', 8)
    };
    let u32_read = |p| -> Option<u32> {
        let bytes = data.get(p..p + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    for _ in 0..100_000 {
        let code = data.get(p..p + 4).ok_or(PreviewError::Unsupported)?;
        let header = if wide { 32 } else { 16 + pointer };
        let length = if wide {
            let bytes = data
                .get(p + 16..p + 24)
                .ok_or(PreviewError::InvalidModel)?
                .try_into()
                .unwrap();
            let value = if big {
                u64::from_be_bytes(bytes)
            } else {
                u64::from_le_bytes(bytes)
            };
            usize::try_from(value).map_err(|_| PreviewError::TooLarge)?
        } else {
            u32_read(p + 4).ok_or(PreviewError::InvalidModel)? as usize
        };
        p += header;
        if code == b"DNA1" || code == b"ENDB" {
            return Err(PreviewError::Unsupported);
        }
        let end = p.checked_add(length).ok_or(PreviewError::TooLarge)?;
        if end > data.len() {
            return Err(PreviewError::Unsupported);
        }
        if code == b"TEST" {
            let width = u32_read(p).ok_or(PreviewError::InvalidModel)?;
            let height = u32_read(p + 4).ok_or(PreviewError::InvalidModel)?;
            if width == 0 || height == 0 || width > 1024 || height > 1024 {
                return Err(PreviewError::InvalidModel);
            }
            let length = width as usize * height as usize * 4;
            let pixels = data
                .get(p + 8..p + 8 + length)
                .filter(|_| p + 8 + length <= end)
                .ok_or(PreviewError::InvalidModel)?;
            let mut rgba = image::RgbaImage::new(width, height);
            for y in 0..height {
                let row = ((height - y - 1) * width * 4) as usize;
                for x in 0..width {
                    let i = row + x as usize * 4;
                    rgba.put_pixel(x, y, image::Rgba(pixels[i..i + 4].try_into().unwrap()));
                }
            }
            let mut png = Cursor::new(Vec::new());
            rgba.write_to(&mut png, image::ImageFormat::Png)
                .map_err(|_| PreviewError::Png)?;
            return images::normalize(png.get_ref());
        }
        p = end;
    }
    Err(PreviewError::TooLarge)
}

pub(super) fn load_bytes(extension: &str, data: &[u8]) -> Result<Vec<u8>, PreviewError> {
    match extension {
        "blend" | "blend1" => blender(data),
        "skp" => sketchup(data),
        "3dm" => rhino(data),
        "sldprt" | "sldasm" | "slddrw" | "ipt" | "iam" | "ipn" | "idw" => {
            if data.starts_with(CFB_MAGIC)
                && let Ok(png) = compound(Cursor::new(data))
            {
                return Ok(png);
            }
            solidworks(data)
                .or_else(|| embedded_png(data, false))
                .ok_or(PreviewError::Unsupported)
        }
        _ => Err(PreviewError::Unsupported),
    }
}

pub(super) fn load_file(path: &Path, extension: &str) -> Result<Vec<u8>, PreviewError> {
    let mut file = std::fs::File::open(path).map_err(|_| PreviewError::InvalidModel)?;
    let mut magic = [0; 8];
    let count = file
        .read(&mut magic)
        .map_err(|_| PreviewError::InvalidModel)?;
    file.rewind().map_err(|_| PreviewError::InvalidModel)?;
    if count == 8 && magic == CFB_MAGIC {
        if file
            .metadata()
            .map_err(|_| PreviewError::InvalidModel)?
            .len()
            > 512 << 20
        {
            return Err(PreviewError::TooLarge);
        }
        if let Ok(png) = compound(&mut file) {
            return Ok(png);
        }
        file.rewind().map_err(|_| PreviewError::InvalidModel)?;
    }
    let mut prefix = Vec::new();
    file.take(PREFIX_BYTES as u64)
        .read_to_end(&mut prefix)
        .map_err(|_| PreviewError::InvalidModel)?;
    load_bytes(extension, &prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn png(width: usize, height: usize) -> Vec<u8> {
        super::super::encode_png(&[20, 80, 120].repeat(width * height), (width, height)).unwrap()
    }

    fn dib_fixture() -> Vec<u8> {
        let mut data = vec![0; 40];
        data[..4].copy_from_slice(&40u32.to_le_bytes());
        data[4..8].copy_from_slice(&2u32.to_le_bytes());
        data[8..12].copy_from_slice(&2u32.to_le_bytes());
        data[12..14].copy_from_slice(&1u16.to_le_bytes());
        data[14..16].copy_from_slice(&24u16.to_le_bytes());
        data.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0]);
        data
    }

    #[test]
    fn reads_plain_gzip_and_new_blender_headers_with_correct_alpha_and_rows() {
        for (wide, big, pointer) in [(false, false, 4), (false, true, 8), (true, false, 8)] {
            let mut data = if wide {
                b"BLENDER17-01v0500".to_vec()
            } else {
                let mut header = b"BLENDER".to_vec();
                header.push(if pointer == 4 { b'_' } else { b'-' });
                header.push(if big { b'V' } else { b'v' });
                header.extend_from_slice(b"300");
                header
            };
            data.extend_from_slice(b"TEST");
            if wide {
                data.extend_from_slice(&[0; 12]);
                data.extend_from_slice(&24u64.to_le_bytes());
                data.extend_from_slice(&1u64.to_le_bytes());
            } else {
                data.extend_from_slice(&if big {
                    24u32.to_be_bytes()
                } else {
                    24u32.to_le_bytes()
                });
                data.extend_from_slice(&vec![0; pointer + 8]);
            }
            for _ in 0..2 {
                data.extend_from_slice(&if big {
                    2u32.to_be_bytes()
                } else {
                    2u32.to_le_bytes()
                });
            }
            data.extend_from_slice(&[
                255, 0, 0, 255, 255, 0, 0, 255, 0, 255, 0, 128, 0, 255, 0, 128,
            ]);
            let result = load_bytes("blend", &data).unwrap();
            let image = image::load_from_memory(&result).unwrap().to_rgba8();
            assert_eq!(image.get_pixel(0, 0).0, [0, 255, 0, 128]);
            let mut gzip =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(&data).unwrap();
            assert_eq!(
                load_bytes("blend1", &gzip.finish().unwrap()).unwrap(),
                result
            );
        }
        assert!(load_bytes("blend", b"\x28\xb5\x2f\xfd").is_err());
    }

    #[test]
    fn sketchup_uses_document_preview_and_generic_search_skips_broken_images() {
        let thumbnail = png(2, 2);
        let texture = png(8, 8);
        let mut data = b"SketchUp CDib".to_vec();
        data.extend_from_slice(&thumbnail);
        data.extend_from_slice(&texture);
        assert_eq!(load_bytes("skp", &data).unwrap(), thumbnail);
        let mut data = b"broken \x89PNG\r\n\x1a\n".to_vec();
        data.extend_from_slice(&texture);
        assert_eq!(load_bytes("sldprt", &data).unwrap(), texture);
        assert!(load_bytes("ipt", b"no stored preview").is_err());
    }

    #[test]
    fn reads_solidworks_compressed_preview_records() {
        let thumbnail = png(3, 2);
        let mut compressor =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        compressor.write_all(&thumbnail).unwrap();
        let compressed = compressor.finish().unwrap();
        let name = b"PreviewPNG";
        let mut data = vec![0x14, 0, 6, 0, 8, 0];
        data.extend_from_slice(&[0; 8]);
        for value in [compressed.len(), thumbnail.len(), name.len()] {
            data.extend_from_slice(&(value as u32).to_le_bytes());
        }
        data.extend(name.iter().map(|b| b.rotate_left(4)));
        data.extend(compressed);
        for ext in ["sldprt", "sldasm", "slddrw"] {
            assert_eq!(load_bytes(ext, &data).unwrap(), thumbnail);
        }
    }

    #[test]
    fn reads_ole_preview_and_summary_dib_streams() {
        let thumbnail = png(3, 2);
        let mut cfb = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cfb.create_stream("/PreviewPNG")
            .unwrap()
            .write_all(&thumbnail)
            .unwrap();
        let data = cfb.into_inner().into_inner();
        for ext in ["ipt", "iam", "ipn", "idw"] {
            assert_eq!(load_bytes(ext, &data).unwrap(), thumbnail);
        }

        let bitmap = dib_fixture();
        let mut summary = vec![0; 80];
        summary[44..48].copy_from_slice(&48u32.to_le_bytes());
        summary[52..56].copy_from_slice(&1u32.to_le_bytes());
        summary[56..60].copy_from_slice(&0x11u32.to_le_bytes());
        summary[60..64].copy_from_slice(&16u32.to_le_bytes());
        summary[64..68].copy_from_slice(&0x47u32.to_le_bytes());
        summary[68..72].copy_from_slice(&((bitmap.len() + 8) as u32).to_le_bytes());
        summary[72..76].copy_from_slice(&u32::MAX.to_le_bytes());
        summary[76..80].copy_from_slice(&8u32.to_le_bytes());
        summary.extend_from_slice(&bitmap);
        let mut cfb = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cfb.create_stream("/\u{5}SummaryInformation")
            .unwrap()
            .write_all(&summary)
            .unwrap();
        let png = load_bytes("ipt", &cfb.into_inner().into_inner()).unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgb8();
        assert_eq!(image.get_pixel(0, 0).0, [0, 255, 0]);
    }

    #[test]
    fn reads_rhino_compressed_and_plain_dib_previews() {
        let bitmap = dib_fixture();
        let expected = dib(&bitmap, 0).unwrap();
        for compressed in [false, true] {
            let mut data = b"3D Geometry File Format".to_vec();
            data.extend_from_slice(
                &if compressed {
                    0x20008025u32
                } else {
                    0x20008023
                }
                .to_le_bytes(),
            );
            data.extend_from_slice(&0u32.to_le_bytes());
            if compressed {
                data.extend_from_slice(&bitmap[..40]);
                let mut zlib =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                zlib.write_all(&bitmap[40..]).unwrap();
                data.extend(zlib.finish().unwrap());
            } else {
                data.extend_from_slice(&bitmap);
            }
            assert_eq!(load_bytes("3dm", &data).unwrap(), expected);
        }
        let mut oversized = bitmap;
        oversized[8..12].copy_from_slice(&i32::MIN.to_le_bytes());
        assert!(dib(&oversized, 0).is_none());
    }
}
