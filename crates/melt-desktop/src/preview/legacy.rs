//! Static VRML 1/2, XML X3D and 3DS meshes. No scripts or external resources run.
use super::{Point, PreviewError, Triangle, mesh, scene};
use std::collections::HashMap;
use vek::{Mat4, Vec3};

#[derive(Default)]
struct Node {
    kind: String,
    numbers: HashMap<String, Vec<f32>>,
    fields: HashMap<String, Vec<usize>>,
    children: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum Token<'a> {
    Word(&'a str),
    Number(f32),
    Open,
    Close,
    List,
    EndList,
    String,
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    position: usize,
    nodes: Vec<Node>,
    defs: HashMap<String, usize>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Result<Self, PreviewError> {
        let bytes = text.as_bytes();
        let mut tokens = Vec::new();
        let mut p = 0;
        while p < bytes.len() {
            let c = bytes[p];
            match c {
                b'#' => {
                    while p < bytes.len() && bytes[p] != b'\n' {
                        p += 1;
                    }
                }
                b' ' | b'\t' | b'\r' | b'\n' | b',' => p += 1,
                b'{' | b'}' | b'[' | b']' => {
                    tokens.push(match c {
                        b'{' => Token::Open,
                        b'}' => Token::Close,
                        b'[' => Token::List,
                        _ => Token::EndList,
                    });
                    p += 1;
                }
                b'"' => {
                    p += 1;
                    while p < bytes.len() && bytes[p] != b'"' {
                        if bytes[p] == b'\\' {
                            p += 1;
                        }
                        p += 1;
                    }
                    if p >= bytes.len() {
                        return Err(PreviewError::InvalidModel);
                    }
                    p += 1;
                    tokens.push(Token::String);
                }
                _ => {
                    let start = p;
                    while p < bytes.len()
                        && !bytes[p].is_ascii_whitespace()
                        && !b"{}[],#\"".contains(&bytes[p])
                    {
                        p += 1;
                    }
                    if p == start {
                        return Err(PreviewError::InvalidModel);
                    }
                    let word = &text[start..p];
                    tokens.push(if let Ok(number) = word.parse::<f32>() {
                        if !number.is_finite() {
                            return Err(PreviewError::InvalidModel);
                        }
                        Token::Number(number)
                    } else {
                        Token::Word(word)
                    });
                }
            }
            if tokens.len() > mesh::MAX_VERTICES * 3 {
                return Err(PreviewError::TooLarge);
            }
        }
        Ok(Self {
            tokens,
            position: 0,
            nodes: Vec::new(),
            defs: HashMap::new(),
        })
    }

    fn peek(&self) -> Option<Token<'a>> {
        self.tokens.get(self.position).copied()
    }
    fn next(&mut self) -> Result<Token<'a>, PreviewError> {
        let token = self.peek().ok_or(PreviewError::InvalidModel)?;
        self.position += 1;
        Ok(token)
    }
    fn word(&mut self) -> Result<&'a str, PreviewError> {
        if let Token::Word(word) = self.next()? {
            Ok(word)
        } else {
            Err(PreviewError::InvalidModel)
        }
    }
    fn is_node(&self) -> bool {
        matches!(self.peek(), Some(Token::Word("DEF" | "USE")))
            || matches!(
                (self.peek(), self.tokens.get(self.position + 1)),
                (Some(Token::Word(_)), Some(Token::Open))
            )
    }
    fn node(&mut self, depth: usize) -> Result<usize, PreviewError> {
        if depth > 64 || self.nodes.len() >= 100_000 {
            return Err(PreviewError::TooLarge);
        }
        let mut kind = self.word()?;
        if kind == "USE" {
            let name = self.word()?;
            return self
                .defs
                .get(name)
                .copied()
                .ok_or(PreviewError::InvalidModel);
        }
        let def = if kind == "DEF" {
            let def = self.word()?;
            kind = self.word()?;
            Some(def)
        } else {
            None
        };
        if self.next()? != Token::Open {
            return Err(PreviewError::InvalidModel);
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind: kind.to_owned(),
            ..Node::default()
        });
        if let Some(def) = def {
            self.defs.insert(def.to_owned(), id);
        }
        loop {
            if self.peek() == Some(Token::Close) {
                self.position += 1;
                break;
            }
            if self.is_node() {
                let child = self.node(depth + 1)?;
                self.nodes[id].children.push(child);
                continue;
            }
            let field = self.word()?;
            let mut numbers = Vec::new();
            let mut children = Vec::new();
            if self.peek() == Some(Token::List) {
                self.position += 1;
                while self.peek() != Some(Token::EndList) {
                    if self.is_node() {
                        children.push(self.node(depth + 1)?);
                    } else if let Token::Number(value) = self.next()? {
                        numbers.push(value);
                    }
                }
                self.position += 1;
            } else if self.is_node() {
                children.push(self.node(depth + 1)?);
            } else if matches!(self.peek(), Some(Token::Number(_))) {
                while let Some(Token::Number(value)) = self.peek() {
                    numbers.push(value);
                    self.position += 1;
                }
            } else {
                self.next()?;
            }
            if !numbers.is_empty() {
                self.nodes[id].numbers.insert(field.to_owned(), numbers);
            }
            if !children.is_empty() {
                self.nodes[id].fields.insert(field.to_owned(), children);
            }
        }
        Ok(id)
    }
}

fn values<'a>(node: &'a Node, field: &str) -> &'a [f32] {
    node.numbers
        .get(field)
        .map(Vec::as_slice)
        .unwrap_or_default()
}
fn vector(node: &Node, field: &str, fallback: [f32; 3]) -> Result<Vec3<f32>, PreviewError> {
    let v = values(node, field);
    match v {
        [] => Ok(Vec3::from(fallback)),
        [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
        _ => Err(PreviewError::InvalidModel),
    }
}
fn local(node: &Node, v1: bool) -> Result<Mat4<f32>, PreviewError> {
    let translation = vector(node, "translation", [0.; 3])?;
    let center = vector(node, "center", [0.; 3])?;
    let scale = vector(node, if v1 { "scaleFactor" } else { "scale" }, [1.; 3])?;
    let rotation = if values(node, "rotation").is_empty() {
        Mat4::<f32>::identity()
    } else {
        scene::rotation(values(node, "rotation"))?
    };
    let sr = if values(node, "scaleOrientation").is_empty() {
        Mat4::<f32>::identity()
    } else {
        scene::rotation(values(node, "scaleOrientation"))?
    };
    Ok(Mat4::<f32>::translation_3d(translation + center)
        * rotation
        * sr
        * Mat4::<f32>::scaling_3d(scale)
        * sr.transposed()
        * Mat4::<f32>::translation_3d(-center))
}

fn points(values: &[f32]) -> Result<Vec<Point>, PreviewError> {
    if !values.len().is_multiple_of(3) || values.len() / 3 > mesh::MAX_VERTICES {
        return Err(PreviewError::InvalidModel);
    }
    Ok(values
        .as_chunks::<3>()
        .0
        .iter()
        .map(|v| [v[0], v[1], v[2]])
        .collect())
}

fn geometry(node: &Node, vertices: &[Point]) -> Result<Vec<Triangle>, PreviewError> {
    let mut out = Vec::new();
    match node.kind.as_str() {
        "IndexedFaceSet" => {
            let mut polygon = Vec::new();
            for value in values(node, "coordIndex")
                .iter()
                .copied()
                .chain(std::iter::once(-1.))
            {
                if value == -1. {
                    if !polygon.is_empty() {
                        mesh::indexed(vertices, &polygon, &mut out)?;
                        polygon.clear();
                    }
                } else if value < 0. || value.fract() != 0. {
                    return Err(PreviewError::InvalidModel);
                } else {
                    polygon.push(value as usize);
                }
            }
        }
        "IndexedTriangleSet" => {
            let indices = values(node, "index");
            if !indices.len().is_multiple_of(3)
                || indices.iter().any(|v| *v < 0. || v.fract() != 0.)
            {
                return Err(PreviewError::InvalidModel);
            }
            for indices in indices.as_chunks::<3>().0.iter() {
                mesh::indexed(
                    vertices,
                    &[
                        indices[0] as usize,
                        indices[1] as usize,
                        indices[2] as usize,
                    ],
                    &mut out,
                )?;
            }
        }
        "TriangleSet" => {
            for i in (0..vertices.len()).step_by(3) {
                mesh::indexed(vertices, &[i, i + 1, i + 2], &mut out)?;
            }
        }
        _ => {}
    }
    Ok(out)
}

fn children(node: &Node) -> Vec<usize> {
    node.fields
        .get("children")
        .into_iter()
        .flatten()
        .chain(&node.children)
        .copied()
        .collect()
}

struct Walker<'a> {
    nodes: &'a [Node],
    out: Vec<Triangle>,
    visits: usize,
}
impl Walker<'_> {
    fn check(&mut self, depth: usize) -> Result<(), PreviewError> {
        self.visits += 1;
        if depth > 64 || self.visits > 100_000 {
            Err(PreviewError::TooLarge)
        } else {
            Ok(())
        }
    }
    fn walk(&mut self, id: usize, parent: Mat4<f32>, depth: usize) -> Result<(), PreviewError> {
        self.check(depth)?;
        let node = &self.nodes[id];
        let world = if matches!(node.kind.as_str(), "Transform" | "CADPart") {
            parent * local(node, false)?
        } else {
            parent
        };
        match node.kind.as_str() {
            "Shape" => {
                let id = node
                    .fields
                    .get("geometry")
                    .and_then(|v| v.first())
                    .copied()
                    .or_else(|| {
                        node.children.iter().copied().find(|id| {
                            matches!(
                                self.nodes[*id].kind.as_str(),
                                "IndexedFaceSet" | "IndexedTriangleSet" | "TriangleSet"
                            )
                        })
                    });
                if let Some(id) = id {
                    let geometry_node = &self.nodes[id];
                    let coord = geometry_node
                        .fields
                        .get("coord")
                        .and_then(|v| v.first())
                        .copied()
                        .or_else(|| {
                            geometry_node.children.iter().copied().find(|id| {
                                matches!(
                                    self.nodes[*id].kind.as_str(),
                                    "Coordinate" | "CoordinateDouble"
                                )
                            })
                        });
                    if let Some(coord) = coord {
                        let vertices = points(values(&self.nodes[coord], "point"))?;
                        scene::append(&mut self.out, &geometry(geometry_node, &vertices)?, world)?;
                    }
                }
            }
            "Switch" => {
                let choice = values(node, "whichChoice").first().copied().unwrap_or(-1.);
                let kids = node
                    .fields
                    .get("choice")
                    .cloned()
                    .unwrap_or_else(|| children(node));
                if choice >= 0.
                    && let Some(id) = kids.get(choice as usize)
                {
                    self.walk(*id, world, depth + 1)?;
                }
            }
            "LOD" => {
                let kids = node
                    .fields
                    .get("level")
                    .cloned()
                    .unwrap_or_else(|| children(node));
                if let Some(id) = kids.first() {
                    self.walk(*id, world, depth + 1)?;
                }
            }
            _ => {
                for child in children(node) {
                    self.walk(child, world, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    fn v1(
        &mut self,
        id: usize,
        state: &mut (Vec<Point>, Mat4<f32>),
        depth: usize,
    ) -> Result<(), PreviewError> {
        self.check(depth)?;
        let node = &self.nodes[id];
        match node.kind.as_str() {
            "Separator" | "TransformSeparator" => {
                let mut inner = state.clone();
                for child in &node.children {
                    self.v1(*child, &mut inner, depth + 1)?;
                }
                if node.kind == "TransformSeparator" {
                    state.0 = inner.0;
                }
            }
            "Coordinate3" => state.0 = points(values(node, "point"))?,
            "Translation" => {
                state.1 *= Mat4::<f32>::translation_3d(vector(node, "translation", [0.; 3])?)
            }
            "Scale" => state.1 *= Mat4::<f32>::scaling_3d(vector(node, "scaleFactor", [1.; 3])?),
            "Rotation" => state.1 *= scene::rotation(values(node, "rotation"))?,
            "Transform" => state.1 *= local(node, true)?,
            "MatrixTransform" => {
                let values = values(node, "matrix");
                if values.len() != 16 {
                    return Err(PreviewError::InvalidModel);
                }
                state.1 *= Mat4::<f32>::from_row_arrays(std::array::from_fn(|r| {
                    std::array::from_fn(|c| values[r * 4 + c])
                }));
            }
            "IndexedFaceSet" => scene::append(&mut self.out, &geometry(node, &state.0)?, state.1)?,
            _ => {
                for child in &node.children {
                    self.v1(*child, state, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn vrml(bytes: &[u8]) -> Result<Vec<Triangle>, PreviewError> {
    let text = std::str::from_utf8(bytes).map_err(|_| PreviewError::InvalidModel)?;
    if !text.starts_with("#VRML") {
        return Err(PreviewError::InvalidModel);
    }
    let mut parser = Parser::new(text)?;
    let mut roots = Vec::new();
    while parser.peek().is_some() {
        if parser.peek() == Some(Token::Word("ROUTE")) {
            for _ in 0..4 {
                parser.next()?;
            }
        } else if matches!(parser.peek(), Some(Token::Word("PROTO" | "EXTERNPROTO"))) {
            return Err(PreviewError::Unsupported);
        } else {
            roots.push(parser.node(0)?);
        }
    }
    let mut walker = Walker {
        nodes: &parser.nodes,
        out: Vec::new(),
        visits: 0,
    };
    if text
        .lines()
        .next()
        .is_some_and(|line| line.contains("V1.0"))
    {
        let mut state = (Vec::new(), Mat4::<f32>::identity());
        for id in roots {
            walker.v1(id, &mut state, 0)?;
        }
    } else {
        for id in roots {
            walker.walk(id, Mat4::<f32>::identity(), 0)?;
        }
    }
    scene::y_up(&mut walker.out);
    Ok(walker.out)
}

pub(super) fn x3d(bytes: &[u8]) -> Result<Vec<Triangle>, PreviewError> {
    let doc = scene::xml_document(bytes)?;
    let root = doc
        .descendants()
        .find(|n| n.has_tag_name("Scene"))
        .ok_or(PreviewError::InvalidModel)?;
    fn convert(
        node: roxmltree::Node<'_, '_>,
        nodes: &mut Vec<Node>,
        defs: &mut HashMap<String, usize>,
        depth: usize,
    ) -> Result<usize, PreviewError> {
        if depth > 64 || nodes.len() >= 100_000 {
            return Err(PreviewError::TooLarge);
        }
        if let Some(name) = node.attribute("USE") {
            return defs.get(name).copied().ok_or(PreviewError::InvalidModel);
        }
        let id = nodes.len();
        nodes.push(Node {
            kind: node.tag_name().name().to_owned(),
            ..Node::default()
        });
        if let Some(name) = node.attribute("DEF") {
            defs.insert(name.to_owned(), id);
        }
        for attr in node.attributes() {
            if matches!(
                attr.name(),
                "translation"
                    | "rotation"
                    | "scale"
                    | "center"
                    | "scaleOrientation"
                    | "point"
                    | "coordIndex"
                    | "index"
                    | "whichChoice"
            ) {
                let values: Vec<f32> = scene::numbers(attr.value())?;
                if !values.iter().all(|v| v.is_finite()) {
                    return Err(PreviewError::InvalidModel);
                }
                nodes[id].numbers.insert(attr.name().to_owned(), values);
            }
        }
        for child in node.children().filter(|n| n.is_element()) {
            let child = convert(child, nodes, defs, depth + 1)?;
            nodes[id].children.push(child);
        }
        Ok(id)
    }
    let mut nodes = Vec::new();
    let root = convert(root, &mut nodes, &mut HashMap::new(), 0)?;
    let mut walker = Walker {
        nodes: &nodes,
        out: Vec::new(),
        visits: 0,
    };
    walker.walk(root, Mat4::<f32>::identity(), 0)?;
    scene::y_up(&mut walker.out);
    Ok(walker.out)
}

pub(super) fn studio3ds(bytes: &[u8]) -> Result<Vec<Triangle>, PreviewError> {
    fn u16_at(bytes: &[u8], offset: usize) -> Result<usize, PreviewError> {
        Ok(u16::from_le_bytes(
            bytes
                .get(offset..offset + 2)
                .ok_or(PreviewError::InvalidModel)?
                .try_into()
                .unwrap(),
        ) as usize)
    }
    fn walk(
        bytes: &[u8],
        mut p: usize,
        end: usize,
        depth: usize,
        vertices: &mut Vec<Point>,
        faces: &mut Vec<[usize; 3]>,
        out: &mut Vec<Triangle>,
    ) -> Result<(), PreviewError> {
        if depth > 32 {
            return Err(PreviewError::TooLarge);
        }
        while p < end {
            let id = u16_at(bytes, p)?;
            let length = u32::from_le_bytes(
                bytes
                    .get(p + 2..p + 6)
                    .ok_or(PreviewError::InvalidModel)?
                    .try_into()
                    .unwrap(),
            ) as usize;
            if length < 6 || length > end - p {
                return Err(PreviewError::InvalidModel);
            }
            let body = p + 6;
            let stop = p + length;
            match id {
                0x4d4d | 0x3d3d => walk(bytes, body, stop, depth + 1, vertices, faces, out)?,
                0x4000 => {
                    let n = bytes
                        .get(body..stop)
                        .ok_or(PreviewError::InvalidModel)?
                        .iter()
                        .position(|v| *v == 0)
                        .ok_or(PreviewError::InvalidModel)?;
                    walk(bytes, body + n + 1, stop, depth + 1, vertices, faces, out)?;
                }
                0x4100 => {
                    vertices.clear();
                    faces.clear();
                    walk(bytes, body, stop, depth + 1, vertices, faces, out)?;
                    for face in faces.iter() {
                        mesh::indexed(vertices, face, out)?;
                    }
                }
                0x4110 => {
                    let count = u16_at(bytes, body)?;
                    let data = bytes
                        .get(body + 2..body + 2 + count * 12)
                        .filter(|_| body + 2 + count * 12 <= stop)
                        .ok_or(PreviewError::InvalidModel)?;
                    for v in data.as_chunks::<12>().0.iter() {
                        vertices.push(std::array::from_fn(|c| {
                            f32::from_le_bytes(v[c * 4..c * 4 + 4].try_into().unwrap())
                        }));
                    }
                }
                0x4120 => {
                    let count = u16_at(bytes, body)?;
                    if body + 2 + count * 8 > stop {
                        return Err(PreviewError::InvalidModel);
                    }
                    for i in 0..count {
                        let p = body + 2 + i * 8;
                        faces.push([
                            u16_at(bytes, p)?,
                            u16_at(bytes, p + 2)?,
                            u16_at(bytes, p + 4)?,
                        ]);
                    }
                }
                _ => {}
            }
            p = stop;
        }
        Ok(())
    }
    if bytes.get(..2) != Some(&[0x4d, 0x4d]) {
        return Err(PreviewError::InvalidModel);
    }
    let mut out = Vec::new();
    walk(
        bytes,
        0,
        bytes.len(),
        0,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut out,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vrml_and_x3d_preserve_instances_and_transform_hierarchy() {
        let text = b"#VRML V2.0 utf8\nTransform { translation 5 0 0 children [ DEF s Shape { geometry IndexedFaceSet { coord Coordinate { point [0 0 0,1 0 0,0 1 0] } coordIndex [0 1 2 -1] } } Transform { translation 2 0 0 children [USE s] } ] }";
        let triangles = vrml(text).unwrap();
        assert_eq!(triangles.len(), 2);
        assert_eq!(triangles[0][0][0], 5.);
        assert_eq!(triangles[1][0][0], 7.);
        let xml = br#"<X3D><Scene><Transform translation="5 0 0"><Shape DEF="s"><IndexedFaceSet coordIndex="0 1 2 -1"><Coordinate point="0 0 0 1 0 0 0 1 0"/></IndexedFaceSet></Shape><Transform translation="2 0 0"><Shape USE="s"/></Transform></Transform></Scene></X3D>"#;
        assert_eq!(x3d(xml).unwrap(), triangles);
        assert!(vrml(b"#VRML V2.0 utf8\nDEF a Group { children [USE a] }").is_err());
    }

    #[test]
    fn vrml_one_uses_coordinate_and_transform_state() {
        let text = b"#VRML V1.0 ascii\nSeparator { Translation { translation 3 0 0 } Coordinate3 { point [0 0 0,1 0 0,0 1 0] } IndexedFaceSet { coordIndex [0 1 2 -1] } }";
        assert_eq!(vrml(text).unwrap()[0][0][0], 3.);
        assert!(studio3ds(&[0x4d, 0x4d, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn renders_a_chunked_3ds_mesh() {
        fn chunk(kind: u16, body: &[u8]) -> Vec<u8> {
            let mut out = kind.to_le_bytes().to_vec();
            out.extend_from_slice(&((body.len() + 6) as u32).to_le_bytes());
            out.extend_from_slice(body);
            out
        }
        let mut vertices = 3u16.to_le_bytes().to_vec();
        for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            vertices.extend_from_slice(&value.to_le_bytes());
        }
        let faces = [1u16, 0, 1, 2, 0]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut mesh = chunk(0x4110, &vertices);
        mesh.extend(chunk(0x4120, &faces));
        let mut object = b"mesh\0".to_vec();
        object.extend(chunk(0x4100, &mesh));
        let data = chunk(0x4d4d, &chunk(0x3d3d, &chunk(0x4000, &object)));
        assert_eq!(studio3ds(&data).unwrap().len(), 1);
        assert!(super::super::validate_png(
            &super::super::from_bytes("3ds", &data, (64, 36)).unwrap()
        ));
    }
}
