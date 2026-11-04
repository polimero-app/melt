//! Bounded 3MF mesh/assembly reader, including production-extension model parts.
use super::{MAX_MODEL_XML_BYTES, Point, PreviewError, Triangle, archive};
use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    sync::Arc,
};

const MAX_TRIANGLES: usize = 2_000_000;
const MAX_VERTICES: usize = 4_000_000;
const MAX_INSTANCES: usize = 100_000;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug)]
struct Transform([f32; 12]);

impl Default for Transform {
    fn default() -> Self {
        Self([1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.])
    }
}

impl Transform {
    fn parse(value: Option<&str>) -> Result<Self, PreviewError> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let values: Vec<f32> = value
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| PreviewError::InvalidModel)?;
        let matrix: [f32; 12] = values.try_into().map_err(|_| PreviewError::InvalidModel)?;
        if !matrix.iter().all(|v| v.is_finite()) {
            return Err(PreviewError::InvalidModel);
        }
        Ok(Self(matrix))
    }

    fn apply(self, p: Point) -> Point {
        let m = self.0;
        std::array::from_fn(|c| p[0] * m[c] + p[1] * m[3 + c] + p[2] * m[6 + c] + m[9 + c])
    }

    /// Row-vector convention: apply this transform, then its parent.
    fn then(self, parent: Self) -> Self {
        let a = self.0;
        let b = parent.0;
        let mut out = [0.; 12];
        for row in 0..3 {
            for col in 0..3 {
                out[row * 3 + col] =
                    a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col];
            }
        }
        for col in 0..3 {
            out[9 + col] = a[9] * b[col] + a[10] * b[3 + col] + a[11] * b[6 + col] + b[9 + col];
        }
        Self(out)
    }
}

#[derive(Default)]
struct Object {
    vertices: Vec<Point>,
    indices: Vec<[usize; 3]>,
    components: Vec<Instance>,
}

struct Instance {
    id: u32,
    path: Option<String>,
    transform: Transform,
}

impl Instance {
    fn parse(attrs: &HashMap<String, String>) -> Result<Self, PreviewError> {
        Ok(Self {
            id: number(attrs, "objectid")?,
            path: attrs.get("path").cloned(),
            transform: Transform::parse(attrs.get("transform").map(String::as_str))?,
        })
    }
}

#[derive(Default)]
struct Part {
    objects: HashMap<u32, Object>,
    build: Vec<Instance>,
    has_build: bool,
}

pub(super) fn attributes(tag: &BytesStart<'_>) -> Result<HashMap<String, String>, PreviewError> {
    let mut out = HashMap::new();
    for attr in tag.attributes() {
        let attr = attr.map_err(|_| PreviewError::InvalidModel)?;
        out.insert(
            attr.key.local_name().as_ref().to_owned(),
            attr.normalized_value(XmlVersion::Implicit1_0)
                .map_err(|_| PreviewError::InvalidModel)?
                .into_owned(),
        );
    }
    Ok(out)
}

fn number<T: std::str::FromStr>(
    attrs: &HashMap<String, String>,
    key: &str,
) -> Result<T, PreviewError> {
    attrs
        .get(key)
        .ok_or(PreviewError::InvalidModel)?
        .parse()
        .map_err(|_| PreviewError::InvalidModel)
}

fn parse(xml: &[u8]) -> Result<Part, PreviewError> {
    let mut reader = Reader::from_reader(xml);
    let mut part = Part::default();
    let mut object = None;
    let mut mesh = false;
    let mut in_build = false;
    let mut vertices = 0;
    let mut triangles = 0;
    let mut depth = 0usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|_| PreviewError::InvalidModel)?;
        match &event {
            Event::Start(tag) | Event::Empty(tag) => {
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
                if depth > 64 {
                    return Err(PreviewError::TooLarge);
                }
                // Vertex/triangle hot paths only need a few attributes.
                let attrs = attributes(tag)?;
                match tag.local_name().as_ref() {
                    "object" => {
                        if object.is_some() || part.objects.len() >= MAX_INSTANCES {
                            return Err(PreviewError::InvalidModel);
                        }
                        let id = number(&attrs, "id")?;
                        if part.objects.insert(id, Object::default()).is_some() {
                            return Err(PreviewError::InvalidModel);
                        }
                        object = Some(id);
                    }
                    "mesh" => mesh = true,
                    "vertex" if mesh => {
                        let point: Point = [
                            number(&attrs, "x")?,
                            number(&attrs, "y")?,
                            number(&attrs, "z")?,
                        ];
                        if !point.iter().all(|v| v.is_finite()) {
                            return Err(PreviewError::InvalidModel);
                        }
                        vertices += 1;
                        if vertices > MAX_VERTICES {
                            return Err(PreviewError::TooLarge);
                        }
                        part.objects
                            .get_mut(&object.ok_or(PreviewError::InvalidModel)?)
                            .ok_or(PreviewError::InvalidModel)?
                            .vertices
                            .push(point);
                    }
                    "triangle" if mesh => {
                        triangles += 1;
                        if triangles > MAX_TRIANGLES {
                            return Err(PreviewError::TooLarge);
                        }
                        let indices = [
                            number(&attrs, "v1")?,
                            number(&attrs, "v2")?,
                            number(&attrs, "v3")?,
                        ];
                        part.objects
                            .get_mut(&object.ok_or(PreviewError::InvalidModel)?)
                            .ok_or(PreviewError::InvalidModel)?
                            .indices
                            .push(indices);
                    }
                    "component" => {
                        let obj = part
                            .objects
                            .get_mut(&object.ok_or(PreviewError::InvalidModel)?)
                            .ok_or(PreviewError::InvalidModel)?;
                        if obj.components.len() >= MAX_INSTANCES {
                            return Err(PreviewError::TooLarge);
                        }
                        obj.components.push(Instance::parse(&attrs)?);
                    }
                    "build" => {
                        part.has_build = true;
                        in_build = true;
                    }
                    "item" if in_build => {
                        if part.build.len() >= MAX_INSTANCES {
                            return Err(PreviewError::TooLarge);
                        }
                        if !attrs
                            .get("printable")
                            .is_some_and(|s| s == "0" || s == "false")
                        {
                            part.build.push(Instance::parse(&attrs)?);
                        }
                    }
                    _ => {}
                }
                if matches!(event, Event::Empty(_)) {
                    match tag.local_name().as_ref() {
                        "object" => object = None,
                        "mesh" => mesh = false,
                        "build" => in_build = false,
                        _ => {}
                    }
                }
            }
            Event::End(tag) => {
                depth = depth.checked_sub(1).ok_or(PreviewError::InvalidModel)?;
                match tag.local_name().as_ref() {
                    "object" => object = None,
                    "mesh" => mesh = false,
                    "build" => in_build = false,
                    _ => {}
                }
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
    Ok(part)
}

struct Context<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    parts: HashMap<String, Arc<Part>>,
    xml_remaining: usize,
    visits: usize,
    active: HashSet<(String, u32)>,
    triangles: Vec<Triangle>,
}

impl Context<'_> {
    fn part(&mut self, path: &str) -> Result<Arc<Part>, PreviewError> {
        let key = path.to_ascii_lowercase();
        if let Some(part) = self.parts.get(&key) {
            return Ok(part.clone());
        }
        let data = archive::read(&mut self.zip, path, self.xml_remaining)?;
        self.xml_remaining -= data.len();
        let part = Arc::new(parse(&data)?);
        self.parts.insert(key, part.clone());
        Ok(part)
    }

    fn append(
        &mut self,
        path: &str,
        instance: &Instance,
        parent: Transform,
        depth: usize,
    ) -> Result<(), PreviewError> {
        self.visits += 1;
        if depth > MAX_DEPTH || self.visits > MAX_INSTANCES {
            return Err(PreviewError::TooLarge);
        }
        let path = instance
            .path
            .as_ref()
            .map_or(Ok(path.to_owned()), |target| {
                archive::package_path(path, target)
            })?;
        let key = (path.to_ascii_lowercase(), instance.id);
        if !self.active.insert(key.clone()) {
            return Err(PreviewError::InvalidModel);
        }
        let part = self.part(&path)?;
        let object = part
            .objects
            .get(&instance.id)
            .ok_or(PreviewError::InvalidModel)?;
        let transform = instance.transform.then(parent);
        if self.triangles.len() + object.indices.len() > MAX_TRIANGLES {
            return Err(PreviewError::TooLarge);
        }
        for indices in &object.indices {
            let mut triangle = [[0.; 3]; 3];
            for (index, point) in indices.iter().zip(&mut triangle) {
                *point = transform.apply(
                    *object
                        .vertices
                        .get(*index)
                        .ok_or(PreviewError::InvalidModel)?,
                );
                if !point.iter().all(|v| v.is_finite()) {
                    return Err(PreviewError::InvalidModel);
                }
            }
            self.triangles.push(triangle);
        }
        for child in &object.components {
            self.append(&path, child, transform, depth + 1)?;
        }
        self.active.remove(&key);
        Ok(())
    }
}

pub(super) fn load(bytes: &[u8]) -> Result<Vec<Triangle>, PreviewError> {
    let mut zip = archive::open(Cursor::new(bytes))?;
    let relationship = archive::read(&mut zip, "_rels/.rels", archive::MAX_RELATIONSHIP_BYTES)
        .ok()
        .and_then(|xml| archive::relationships(&xml, "/3dmodel").ok())
        .and_then(|targets| targets.into_iter().next());
    let root = if let Some(target) = relationship {
        archive::package_path("", &target)?
    } else {
        zip.file_names()
            .filter_map(Result::ok)
            .find(|name| name.eq_ignore_ascii_case("3D/3dmodel.model"))
            .or_else(|| {
                zip.file_names()
                    .filter_map(Result::ok)
                    .find(|name| name.to_ascii_lowercase().ends_with(".model"))
            })
            .ok_or(PreviewError::InvalidModel)?
            .into_owned()
    };
    let mut ctx = Context {
        zip,
        parts: HashMap::new(),
        xml_remaining: MAX_MODEL_XML_BYTES as usize,
        visits: 0,
        active: HashSet::new(),
        triangles: Vec::new(),
    };
    let part = ctx.part(&root)?;
    if part.has_build {
        for item in &part.build {
            ctx.append(&root, item, Transform::default(), 0)?;
        }
    } else {
        // Compatibility with simple exporters that omit <build> entirely.
        let mut ids: Vec<_> = part.objects.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            ctx.append(
                &root,
                &Instance {
                    id,
                    path: None,
                    transform: Transform::default(),
                },
                Transform::default(),
                0,
            )?;
        }
    }
    if ctx.triangles.is_empty() {
        return Err(PreviewError::InvalidModel);
    }
    Ok(ctx.triangles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, xml) in parts {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    const MESH: &str = "<model><resources><object id='9'><mesh><vertices><vertex x='0' y='0' z='0'/><vertex x='1' y='0' z='0'/><vertex x='0' y='1' z='0'/></vertices><triangles><triangle v1='0' v2='1' v3='2'/></triangles></mesh></object></resources></model>";

    #[test]
    fn resolves_root_parts_build_selection_and_composed_transforms() {
        let data = package(&[
            (
                "_rels/.rels",
                "<Relationships><Relationship Type='http://example/3dmodel' Target='/3D/root.model'/></Relationships>",
            ),
            (
                "3D/root.model",
                "<model xmlns:p='http://example/production'><resources><object id='1'><components><component objectid='9' p:path='Objects/part.model' transform='2 0 0 0 2 0 0 0 2 3 0 0'/></components></object><object id='2'/></resources><build><item objectid='1' transform='1 0 0 0 1 0 0 0 1 10 0 0'/></build></model>",
            ),
            ("3D/Objects/part.model", MESH),
        ]);
        assert_eq!(
            load(&data).unwrap(),
            vec![[[13., 0., 0.], [15., 0., 0.], [13., 2., 0.]]]
        );
    }

    #[test]
    fn rejects_cycles_missing_parts_invalid_indices_and_nonfinite_geometry() {
        let cycle = "<model><resources><object id='1'><components><component objectid='1'/></components></object></resources><build><item objectid='1'/></build></model>";
        for xml in [
            cycle.to_owned(),
            MESH.replace("v3='2'", "v3='20'"),
            MESH.replace("x='0'", "x='NaN'"),
            "<model><build><item objectid='10'/></build></model>".to_owned(),
            "<model>".to_owned(),
        ] {
            assert!(load(&package(&[("3D/3dmodel.model", &xml)])).is_err());
        }
    }

    #[test]
    fn skips_nonprintable_build_items() {
        let xml = MESH.replace(
            "</model>",
            "<build><item objectid='9' printable='false'/></build></model>",
        );
        assert!(load(&package(&[("3D/3dmodel.model", &xml)])).is_err());
    }
}
