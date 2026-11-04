use super::{MAX_PREVIEW_BYTES, PreviewError, images};
use quick_xml::{Reader, events::Event};
use std::io::{Read, Seek};

pub(super) const MAX_RELATIONSHIP_BYTES: usize = 1 << 20;
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
    let index = zip
        .file_names()
        .position(|candidate| {
            candidate.is_ok_and(|candidate| candidate.eq_ignore_ascii_case(&name))
        })
        .ok_or(PreviewError::InvalidModel)?;
    read_index(zip, index, limit)
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
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PreviewError::InvalidModel)?;
    if bytes.len() > limit {
        return Err(PreviewError::TooLarge);
    }
    Ok(bytes)
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
    let mut candidates = read(&mut zip, "_rels/.rels", MAX_RELATIONSHIP_BYTES)
        .ok()
        .and_then(|xml| relationships(&xml, "/thumbnail").ok())
        .unwrap_or_default();
    let mut fallback: Vec<(u8, String)> = zip
        .file_names()
        .filter_map(Result::ok)
        .filter_map(|name| {
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
            Some((rank, name.into_owned()))
        })
        .collect();
    fallback.sort_by_key(|(rank, _)| *rank);
    candidates.extend(fallback.into_iter().map(|(_, name)| name));
    for name in candidates {
        if let Ok(bytes) = read(&mut zip, &name, MAX_PREVIEW_BYTES)
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
}
