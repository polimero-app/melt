use super::{MAX_PREVIEW_BYTES, PreviewError, images};
use quick_xml::{Reader, events::Event};
use std::io::{Read, Seek};

pub(super) const MAX_RELATIONSHIP_BYTES: usize = 1 << 20;
pub(super) const MAX_IMAGE_CANDIDATES: usize = 256;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;

pub(super) fn open<R: Read + Seek>(reader: R) -> Result<zip::ZipArchive<R>, PreviewError> {
    let zip = zip::ZipArchive::new(reader).map_err(|_| PreviewError::InvalidModel)?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err(PreviewError::TooLarge);
    }
    Ok(zip)
}

pub(super) fn read<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    limit: usize,
) -> Result<Vec<u8>, PreviewError> {
    let name = package_path("", name)?;
    let index = find_index(zip, &name).ok_or(PreviewError::InvalidModel)?;
    read_index(zip, index, limit)
}

fn find_index<R: Read + Seek>(zip: &zip::ZipArchive<R>, name: &str) -> Option<usize> {
    zip.index_for_name(name).or_else(|| {
        zip.file_names().position(|candidate| {
            candidate.is_ok_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
    })
}

pub(super) fn read_index<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    index: usize,
    limit: usize,
) -> Result<Vec<u8>, PreviewError> {
    let entry = zip
        .by_index(index)
        .map_err(|_| PreviewError::InvalidModel)?;
    if entry.size() > limit as u64 {
        return Err(PreviewError::TooLarge);
    }
    let declared = entry.size() as usize;
    let mut bytes = Vec::with_capacity(declared);
    entry
        .take(declared as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PreviewError::InvalidModel)?;
    if bytes.len() > declared {
        return Err(PreviewError::TooLarge);
    }
    Ok(bytes)
}

/// Charge declared output even when decompression/CRC validation fails.
pub(super) fn read_budgeted<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    index: usize,
    limit: usize,
    budget: &mut usize,
) -> Result<Vec<u8>, PreviewError> {
    let declared = zip
        .by_index(index)
        .map_err(|_| PreviewError::InvalidModel)?
        .size();
    if declared > limit.min(*budget) as u64 {
        return Err(PreviewError::TooLarge);
    }
    *budget -= declared as usize;
    read_index(zip, index, declared as usize)
}

/// Resolve package-relative references without ever accessing the filesystem.
pub(super) fn package_path(base: &str, target: &str) -> Result<String, PreviewError> {
    if target.contains([':', '\\', '\0', '?', '#']) {
        return Err(PreviewError::InvalidModel);
    }
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        base.rsplit_once('/')
            .map_or(Vec::new(), |(dir, _)| dir.split('/').collect())
    };
    for part in target.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or(PreviewError::InvalidModel)?;
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(PreviewError::InvalidModel);
    }
    Ok(parts.join("/"))
}

pub(super) fn relationships(xml: &[u8], suffix: &str) -> Result<Vec<String>, PreviewError> {
    if xml.len() > MAX_RELATIONSHIP_BYTES {
        return Err(PreviewError::TooLarge);
    }
    let mut reader = Reader::from_reader(xml);
    let mut targets = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|_| PreviewError::InvalidModel)?
        {
            Event::Start(tag) | Event::Empty(tag)
                if tag.local_name().as_ref() == "Relationship" =>
            {
                let mut kind = String::new();
                let mut target = None;
                let mut external = false;
                for attr in tag.attributes() {
                    let attr = attr.map_err(|_| PreviewError::InvalidModel)?;
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|_| PreviewError::InvalidModel)?;
                    match attr.key.local_name().as_ref() {
                        "Type" => kind = value.into_owned(),
                        "Target" => target = Some(value.into_owned()),
                        "TargetMode" => external = value.eq_ignore_ascii_case("External"),
                        _ => {}
                    }
                }
                if !external
                    && kind.to_ascii_lowercase().ends_with(suffix)
                    && let Some(target) = target
                {
                    targets.push(target);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(targets)
}

/// Relationship targets win. Fall back to thumbnail-named images, plate 1,
/// and finally arbitrary images, excluding known slicer helper masks.
pub fn extract_3mf_thumbnail(reader: impl Read + Seek) -> Option<Vec<u8>> {
    let mut zip = open(reader).ok()?;
    let relationships = read(&mut zip, "_rels/.rels", MAX_RELATIONSHIP_BYTES)
        .ok()
        .and_then(|xml| relationships(&xml, "/thumbnail").ok())
        .unwrap_or_default();
    let mut candidates: Vec<usize> = relationships
        .into_iter()
        .take(MAX_IMAGE_CANDIDATES)
        .filter_map(|name| {
            package_path("", &name)
                .ok()
                .and_then(|name| find_index(&zip, &name))
        })
        .collect();
    let mut fallback: Vec<(u8, usize)> = zip
        .file_names()
        .enumerate()
        .filter_map(|(index, name)| name.ok().map(|name| (index, name)))
        .filter_map(|(index, name)| {
            let lower = name.to_ascii_lowercase();
            if ![".png", ".jpg", ".jpeg"]
                .iter()
                .any(|ext| lower.ends_with(ext))
                || ["pick", "_no_light", "top_"]
                    .iter()
                    .any(|mask| lower.contains(mask))
            {
                return None;
            }
            let rank = if lower.contains("thumbnail") {
                0
            } else if lower.contains("plate_1.") {
                1
            } else if lower.starts_with("metadata/") {
                2
            } else {
                3
            };
            Some((rank, index))
        })
        .collect();
    fallback.sort_by_key(|(rank, _)| *rank);
    candidates.extend(fallback.into_iter().map(|(_, index)| index));
    let mut budget = 64 << 20;
    for index in candidates.into_iter().take(MAX_IMAGE_CANDIDATES) {
        if let Ok(bytes) = read_budgeted(&mut zip, index, MAX_PREVIEW_BYTES, &mut budget)
            && let Ok(png) = images::normalize(&bytes)
        {
            return Some(png);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn follows_relationships_and_skips_invalid_candidates() {
        let png = super::super::encode_png(&[20, 80, 120], (1, 1)).unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("Metadata/thumbnail.png", b"broken".as_slice()),
            ("custom/preview.png", png.as_slice()),
            ("_rels/.rels", br#"<Relationships><Relationship Type="http://example/thumbnail" Target="/custom/preview.png"/></Relationships>"#),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        let data = zip.finish().unwrap().into_inner();
        assert_eq!(extract_3mf_thumbnail(Cursor::new(data)), Some(png));
        assert!(package_path("3D/root.model", "../../outside").is_err());
        assert!(package_path("", "https://example/image.png").is_err());
        assert_eq!(
            package_path("3D/root.model", "Objects/part.model").unwrap(),
            "3D/Objects/part.model"
        );
    }

    #[test]
    fn failed_crc_checks_still_consume_the_archive_budget() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("bad.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"eight123").unwrap();
        let mut bytes = zip.finish().unwrap().into_inner();
        let offset = open(Cursor::new(&bytes))
            .unwrap()
            .by_index(0)
            .unwrap()
            .data_start()
            .unwrap() as usize;
        bytes[offset] ^= 1;
        let mut zip = open(Cursor::new(bytes)).unwrap();
        let mut budget = 8;
        assert!(read_budgeted(&mut zip, 0, MAX_PREVIEW_BYTES, &mut budget).is_err());
        assert_eq!(budget, 0);
        assert!(matches!(
            read_budgeted(&mut zip, 0, MAX_PREVIEW_BYTES, &mut budget),
            Err(PreviewError::TooLarge)
        ));
    }
}
