use super::{MAX_MODEL_XML_BYTES, Point, PreviewError, Triangle, archive};
use quick_xml::{Reader, events::Event};
use std::io::Cursor;

pub(super) const MAX_TRIANGLES: usize = 2_000_000;
pub(super) const MAX_VERTICES: usize = 4_000_000;

pub(super) fn indexed(
    vertices: &[Point],
    indices: &[usize],
    out: &mut Vec<Triangle>,
) -> Result<(), PreviewError> {
    if indices.len() < 3 {
        return Err(PreviewError::InvalidModel);
    }
    if out.len().saturating_add(indices.len() - 2) > MAX_TRIANGLES {
        return Err(PreviewError::TooLarge);
    }
    for i in 1..indices.len() - 1 {
        let mut triangle = [[0.; 3]; 3];
        for (slot, index) in triangle
            .iter_mut()
            .zip([indices[0], indices[i], indices[i + 1]])
        {
            *slot = *vertices.get(index).ok_or(PreviewError::InvalidModel)?;
            if !slot.iter().all(|v| v.is_finite()) {
                return Err(PreviewError::InvalidModel);
            }
        }
        out.push(triangle);
    }
    Ok(())
}

fn number<T: std::str::FromStr>(text: Option<&str>) -> Result<T, PreviewError> {
    text.ok_or(PreviewError::InvalidModel)?
        .parse()
        .map_err(|_| PreviewError::InvalidModel)
}

pub(super) fn off(bytes: &[u8]) -> Result<Vec<Triangle>, PreviewError> {
    let text = std::str::from_utf8(bytes).map_err(|_| PreviewError::InvalidModel)?;
    let mut lines = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty());
    let header = lines.next().ok_or(PreviewError::InvalidModel)?;
    let mut tokens = header.split_whitespace();
    if !matches!(tokens.next(), Some("OFF" | "COFF" | "NOFF" | "CNOFF")) {
        return Err(PreviewError::Unsupported);
    }
    let counts = tokens.collect::<Vec<_>>().join(" ");
    let mut counts = if counts.is_empty() {
        lines.next().ok_or(PreviewError::InvalidModel)?
    } else {
        &counts
    }
    .split_whitespace();
    let nv: usize = number(counts.next())?;
    let nf: usize = number(counts.next())?;
    if nv > MAX_VERTICES || nf > MAX_TRIANGLES {
        return Err(PreviewError::TooLarge);
    }
    let mut vertices = Vec::new();
    for _ in 0..nv {
        let mut values = lines
            .next()
            .ok_or(PreviewError::InvalidModel)?
            .split_whitespace();
        vertices.push([
            number(values.next())?,
            number(values.next())?,
            number(values.next())?,
        ]);
    }
    let mut triangles = Vec::new();
    for _ in 0..nf {
        let mut values = lines
            .next()
            .ok_or(PreviewError::InvalidModel)?
            .split_whitespace();
        let count: usize = number(values.next())?;
        if count > MAX_VERTICES {
            return Err(PreviewError::TooLarge);
        }
        let indices: Vec<usize> = values
            .take(count)
            .map(|value| number(Some(value)))
            .collect::<Result<_, _>>()?;
        if indices.len() != count {
            return Err(PreviewError::InvalidModel);
        }
        indexed(&vertices, &indices, &mut triangles)?;
    }
    Ok(triangles)
}

pub(super) fn amf(bytes: &[u8]) -> Result<Vec<Triangle>, PreviewError> {
    let xml = if bytes.starts_with(b"PK") {
        let mut zip = archive::open(Cursor::new(bytes))?;
        let name = zip
            .file_names()
            .filter_map(Result::ok)
            .find(|name| {
                name.to_ascii_lowercase().ends_with(".amf")
                    || name.to_ascii_lowercase().ends_with(".xml")
            })
            .ok_or(PreviewError::InvalidModel)?
            .into_owned();
        archive::read(&mut zip, &name, MAX_MODEL_XML_BYTES as usize)?
    } else {
        bytes.to_vec()
    };
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut vertices = Vec::new();
    let mut faces = Vec::<[usize; 3]>::new();
    let mut point = [None; 3];
    let mut indices = [None; 3];
    let mut coordinate = false;
    let mut triangle = false;
    let mut field = None;
    let mut out = Vec::new();
    let mut depth = 0usize;
    loop {
        match reader
            .read_event()
            .map_err(|_| PreviewError::InvalidModel)?
        {
            Event::Start(tag) => {
                depth += 1;
                if depth > 64 {
                    return Err(PreviewError::TooLarge);
                }
                match tag.local_name().as_ref() {
                    "object" => {
                        vertices.clear();
                        faces.clear();
                    }
                    "coordinates" => {
                        coordinate = true;
                        point = [None; 3];
                    }
                    "triangle" => {
                        triangle = true;
                        indices = [None; 3];
                    }
                    "x" | "v1" => field = Some(0),
                    "y" | "v2" => field = Some(1),
                    "z" | "v3" => field = Some(2),
                    _ => {}
                }
            }
            Event::Text(text) => {
                if let Some(field) = field {
                    let text = text.as_ref().trim();
                    if coordinate {
                        point[field] = Some(number(Some(text))?);
                    } else if triangle {
                        indices[field] = Some(number(Some(text))?);
                    }
                }
            }
            Event::End(tag) => {
                depth = depth.checked_sub(1).ok_or(PreviewError::InvalidModel)?;
                match tag.local_name().as_ref() {
                    "coordinates" => {
                        let [Some(x), Some(y), Some(z)] = point else {
                            return Err(PreviewError::InvalidModel);
                        };
                        if vertices.len() >= MAX_VERTICES {
                            return Err(PreviewError::TooLarge);
                        }
                        vertices.push([x, y, z]);
                        coordinate = false;
                    }
                    "triangle" => {
                        let [Some(a), Some(b), Some(c)] = indices else {
                            return Err(PreviewError::InvalidModel);
                        };
                        if faces.len() >= MAX_TRIANGLES {
                            return Err(PreviewError::TooLarge);
                        }
                        faces.push([a, b, c]);
                        triangle = false;
                    }
                    "object" => {
                        for indices in &faces {
                            indexed(&vertices, indices, &mut out)?;
                        }
                    }
                    _ => {}
                }
                field = None;
            }
            Event::DocType(_) => return Err(PreviewError::InvalidModel),
            Event::Eof => {
                if depth != 0 {
                    return Err(PreviewError::InvalidModel);
                }
                break;
            }
            _ => {}
        }
    }
    Ok(out)
}

#[derive(Default)]
struct PlyElement {
    point: [Option<f32>; 3],
    indices: Vec<usize>,
    invalid: bool,
}

impl ply_rs::ply::PropertyAccess for PlyElement {
    fn new() -> Self {
        Self::default()
    }
    fn set_property(&mut self, name: String, property: ply_rs::ply::Property) {
        use ply_rs::ply::Property::*;
        let field = match name.as_str() {
            "x" => Some(0),
            "y" => Some(1),
            "z" => Some(2),
            _ => None,
        };
        if let Some(field) = field {
            self.point[field] = match property {
                Float(v) => Some(v),
                Double(v) => Some(v as f32),
                Char(v) => Some(v as f32),
                UChar(v) => Some(v as f32),
                Short(v) => Some(v as f32),
                UShort(v) => Some(v as f32),
                Int(v) => Some(v as f32),
                UInt(v) => Some(v as f32),
                _ => None,
            };
        } else if matches!(name.as_str(), "vertex_indices" | "vertex_index") {
            let indices: Vec<i64> = match property {
                ListInt(v) => v.into_iter().map(i64::from).collect(),
                ListUInt(v) => v.into_iter().map(i64::from).collect(),
                ListShort(v) => v.into_iter().map(i64::from).collect(),
                ListUShort(v) => v.into_iter().map(i64::from).collect(),
                ListChar(v) => v.into_iter().map(i64::from).collect(),
                ListUChar(v) => v.into_iter().map(i64::from).collect(),
                _ => {
                    self.invalid = true;
                    return;
                }
            };
            if indices.len() > MAX_VERTICES || indices.iter().any(|i| *i < 0) {
                self.invalid = true;
            } else {
                self.indices = indices.into_iter().map(|i| i as usize).collect();
            }
        }
    }
}

pub(super) fn ply(bytes: &[u8]) -> Result<(Vec<Point>, Vec<Triangle>), PreviewError> {
    let parser = ply_rs::parser::Parser::<PlyElement>::new();
    let mut reader = Cursor::new(bytes);
    let header = parser
        .read_header(&mut reader)
        .map_err(|_| PreviewError::InvalidModel)?;
    let count = header
        .elements
        .values()
        .try_fold(0usize, |total, element| total.checked_add(element.count))
        .ok_or(PreviewError::TooLarge)?;
    if count > MAX_VERTICES + MAX_TRIANGLES {
        return Err(PreviewError::TooLarge);
    }
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    for (name, element) in &header.elements {
        if element.properties.is_empty() && element.count != 0 {
            return Err(PreviewError::InvalidModel);
        }
        let elements = parser
            .read_payload_for_element(&mut reader, element, &header)
            .map_err(|_| PreviewError::InvalidModel)?;
        match name.as_str() {
            "vertex" => {
                if elements.len() > MAX_VERTICES {
                    return Err(PreviewError::TooLarge);
                }
                for element in elements {
                    let [Some(x), Some(y), Some(z)] = element.point else {
                        return Err(PreviewError::InvalidModel);
                    };
                    if ![x, y, z].iter().all(|v| v.is_finite()) {
                        return Err(PreviewError::InvalidModel);
                    }
                    vertices.push([x, y, z]);
                }
            }
            "face" => {
                if elements.len() > MAX_TRIANGLES {
                    return Err(PreviewError::TooLarge);
                }
                for element in elements {
                    if element.invalid {
                        return Err(PreviewError::InvalidModel);
                    }
                    faces.push(element.indices);
                }
            }
            _ => {}
        }
    }
    let mut triangles = Vec::new();
    for face in faces {
        indexed(&vertices, &face, &mut triangles)?;
    }
    Ok((vertices, triangles))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulates_off_and_ply_polygons_and_rejects_bad_indices() {
        let off = b"OFF\n4 1 0\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n";
        assert_eq!(super::off(off).unwrap().len(), 2);
        let ply = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        assert_eq!(super::ply(ply).unwrap().1.len(), 1);
        assert!(super::off(b"OFF\n1 1 0\n0 0 0\n3 0 1 2\n").is_err());
        assert!(super::off(b"OFF\n999999999 0 0\n").is_err());
    }

    #[test]
    fn parses_amf_meshes_with_per_object_indices() {
        let object = "<object><mesh><vertices><vertex><coordinates><x>0</x><y>0</y><z>0</z></coordinates></vertex><vertex><coordinates><x>1</x><y>0</y><z>0</z></coordinates></vertex><vertex><coordinates><x>0</x><y>1</y><z>0</z></coordinates></vertex></vertices><volume><triangle><v1>0</v1><v2>1</v2><v3>2</v3></triangle></volume></mesh></object>";
        assert_eq!(
            amf(format!("<amf>{object}{object}</amf>").as_bytes())
                .unwrap()
                .len(),
            2
        );
        assert!(amf(b"<amf>").is_err());
    }

    #[test]
    fn parses_both_binary_ply_byte_orders() {
        for big_endian in [false, true] {
            let encoding = if big_endian {
                "binary_big_endian"
            } else {
                "binary_little_endian"
            };
            let mut data = format!("ply\nformat {encoding} 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n").into_bytes();
            for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
                data.extend_from_slice(&if big_endian {
                    value.to_be_bytes()
                } else {
                    value.to_le_bytes()
                });
            }
            data.push(3);
            for index in 0i32..3 {
                data.extend_from_slice(&if big_endian {
                    index.to_be_bytes()
                } else {
                    index.to_le_bytes()
                });
            }
            assert_eq!(ply(&data).unwrap().1.len(), 1);
        }
    }
}
