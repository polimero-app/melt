use super::{MAX_MODEL_BYTES, MAX_PREVIEW_BYTES, PreviewError, archive, images, model3mf};
use quick_xml::{Reader, events::Event};
use std::io::{Cursor, Read, Seek};

pub(super) fn zip_thumbnail(
    reader: impl Read + Seek,
    extension: &str,
) -> Result<Vec<u8>, PreviewError> {
    let mut budget = 64 << 20;
    zip_inner(reader, extension, 0, &mut budget)
}

fn zip_inner<R: Read + Seek>(
    reader: R,
    extension: &str,
    depth: usize,
    budget: &mut usize,
) -> Result<Vec<u8>, PreviewError> {
    let mut zip = archive::open(reader)?;
    let mut candidates: Vec<_> = zip
        .file_names()
        .enumerate()
        .filter_map(|(i, name)| {
            let lower = name.ok()?.to_ascii_lowercase();
            let image = [".png", ".jpg", ".jpeg"]
                .iter()
                .any(|suffix| lower.ends_with(suffix));
            let hint = match extension {
                "ufp" => lower.contains("thumbnail") || lower.starts_with("metadata/"),
                "fcstd" => lower.contains("thumbnail"),
                "f3d" | "f3z" => lower.contains("preview") || lower.contains("thumbnail"),
                _ => false,
            };
            (image && hint).then_some(i)
        })
        .collect();
    let mut best = None;
    let mut best_area = 0;
    for index in candidates.drain(..) {
        let limit = (*budget).min(MAX_PREVIEW_BYTES);
        let Ok(bytes) = archive::read_index(&mut zip, index, limit) else {
            continue;
        };
        *budget -= bytes.len();
        if let Ok(png) = images::normalize(&bytes) {
            let area = images::png_area(&png);
            if area > best_area {
                best = Some(png);
                best_area = area;
            }
        }
    }
    if let Some(best) = best {
        return Ok(best);
    }
    if extension == "f3z" && depth == 0 {
        let mut designs = Vec::new();
        for index in 0..zip.len() {
            let entry = zip
                .by_index(index)
                .map_err(|_| PreviewError::InvalidModel)?;
            if entry
                .name()
                .is_ok_and(|name| name.to_ascii_lowercase().ends_with(".f3d"))
            {
                designs.push((entry.size(), index));
            }
        }
        designs.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (_, index) in designs {
            let Ok(bytes) = archive::read_index(&mut zip, index, (*budget).min(MAX_MODEL_BYTES))
            else {
                continue;
            };
            *budget -= bytes.len();
            if let Ok(png) = zip_inner(Cursor::new(bytes), "f3d", depth + 1, budget) {
                return Ok(png);
            }
        }
    }
    Err(PreviewError::Unsupported)
}

pub(super) fn lightburn(bytes: &[u8]) -> Result<Vec<u8>, PreviewError> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut reader = Reader::from_reader(bytes);
    loop {
        match reader
            .read_event()
            .map_err(|_| PreviewError::InvalidModel)?
        {
            Event::Start(tag) | Event::Empty(tag) if tag.local_name().as_ref() == "Thumbnail" => {
                let attrs = model3mf::attributes(&tag)?;
                let data = attrs.get("Source").ok_or(PreviewError::Unsupported)?;
                let bytes = STANDARD
                    .decode(data)
                    .map_err(|_| PreviewError::InvalidModel)?;
                return images::normalize(&bytes);
            }
            Event::Eof => return Err(PreviewError::Unsupported),
            Event::DocType(_) => return Err(PreviewError::InvalidModel),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn selects_largest_valid_preview_and_reads_nested_fusion_designs() {
        let small = super::super::encode_png(&[0; 3], (1, 1)).unwrap();
        let large = super::super::encode_png(&[0; 12], (2, 2)).unwrap();
        let inner = zip(&[
            ("Previews/bad.png", b"bad"),
            ("Previews/small.png", &small),
            ("Previews/large.png", &large),
        ]);
        let outer = zip(&[("design.f3d", &inner)]);
        assert_eq!(zip_thumbnail(Cursor::new(outer), "f3z").unwrap(), large);
        assert_eq!(
            zip_thumbnail(
                Cursor::new(zip(&[("thumbnails/Thumbnail.png", &small)])),
                "fcstd"
            )
            .unwrap(),
            small
        );
        assert!(zip_thumbnail(Cursor::new(zip(&[("texture.png", &small)])), "ufp").is_err());
    }
}
