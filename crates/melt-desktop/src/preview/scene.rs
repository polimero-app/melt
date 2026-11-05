//! Scene adapters: retain node instances/transforms, then feed Melt's renderer.
use super::{
    MAX_MODEL_BYTES, Point, PreviewError, Triangle,
    mesh::{self, MAX_TRIANGLES, MAX_VERTICES},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use vek::{Mat4, Vec3, Vec4};

pub(super) fn transform(point: Point, matrix: Mat4<f32>) -> Result<Point, PreviewError> {
    let p = matrix * Vec4::from_point(Vec3::from(point));
    let result = [p.x, p.y, p.z];
    if result.iter().all(|v| v.is_finite()) {
        Ok(result)
    } else {
        Err(PreviewError::InvalidModel)
    }
}

pub(super) fn append(
    out: &mut Vec<Triangle>,
    triangles: &[Triangle],
    matrix: Mat4<f32>,
) -> Result<(), PreviewError> {
    if out.len().saturating_add(triangles.len()) > MAX_TRIANGLES {
        return Err(PreviewError::TooLarge);
    }
    for triangle in triangles {
        out.push([
            transform(triangle[0], matrix)?,
            transform(triangle[1], matrix)?,
            transform(triangle[2], matrix)?,
        ]);
    }
    Ok(())
}

pub(super) fn y_up(triangles: &mut [Triangle]) {
    for point in triangles.iter_mut().flatten() {
        *point = [point[0], -point[2], point[1]];
    }
}

pub(super) fn gltf(bytes: &[u8], path: Option<&Path>) -> Result<Vec<Triangle>, PreviewError> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|_| PreviewError::InvalidModel)?;
    if gltf.extensions_required().next().is_some() {
        return Err(PreviewError::Unsupported);
    }
    let mut budget = MAX_MODEL_BYTES;
    let mut buffers = Vec::new();
    for buffer in gltf.buffers() {
        if buffer.length() > budget {
            return Err(PreviewError::TooLarge);
        }
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => gltf.blob.clone().ok_or(PreviewError::InvalidModel)?,
            gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => {
                let (header, payload) = uri.split_once(',').ok_or(PreviewError::InvalidModel)?;
                if !header.ends_with(";base64") || payload.len() > (budget * 4 / 3 + 4) {
                    return Err(PreviewError::Unsupported);
                }
                STANDARD
                    .decode(payload)
                    .map_err(|_| PreviewError::InvalidModel)?
            }
            gltf::buffer::Source::Uri(uri) => {
                let path = path.ok_or(PreviewError::Unsupported)?;
                if uri.contains([':', '\\', '%', '?', '#', '\0']) || Path::new(uri).is_absolute() {
                    return Err(PreviewError::Unsupported);
                }
                let dir = path
                    .parent()
                    .ok_or(PreviewError::InvalidModel)?
                    .canonicalize()
                    .map_err(|_| PreviewError::InvalidModel)?;
                let target = dir
                    .join(uri)
                    .canonicalize()
                    .map_err(|_| PreviewError::InvalidModel)?;
                if !target.starts_with(&dir) {
                    return Err(PreviewError::InvalidModel);
                }
                let file = std::fs::File::open(target).map_err(|_| PreviewError::InvalidModel)?;
                if file
                    .metadata()
                    .map_err(|_| PreviewError::InvalidModel)?
                    .len()
                    > budget as u64
                {
                    return Err(PreviewError::TooLarge);
                }
                super::read_bounded(file, budget)?
            }
        };
        if data.len() > budget || data.len() < buffer.length() {
            return Err(PreviewError::TooLarge);
        }
        budget -= data.len();
        buffers.push(data);
    }
    let mut geometries = HashMap::new();
    let mut cached_triangles = 0usize;
    for m in gltf.meshes() {
        let mut triangles = Vec::new();
        for primitive in m.primitives() {
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
            let positions = reader.read_positions().ok_or(PreviewError::Unsupported)?;
            if positions.len() > MAX_VERTICES {
                return Err(PreviewError::TooLarge);
            }
            let vertices: Vec<_> = positions.collect();
            let indices = if let Some(indices) = reader.read_indices() {
                let indices = indices.into_u32();
                if indices.len() > MAX_TRIANGLES * 3 {
                    return Err(PreviewError::TooLarge);
                }
                indices.map(|i| i as usize).collect::<Vec<_>>()
            } else {
                (0..vertices.len()).collect()
            };
            match primitive.mode() {
                gltf::mesh::Mode::Triangles => {
                    if !indices.len().is_multiple_of(3) {
                        return Err(PreviewError::InvalidModel);
                    }
                    for triangle in indices.as_chunks::<3>().0.iter() {
                        mesh::indexed(&vertices, triangle, &mut triangles)?;
                    }
                }
                gltf::mesh::Mode::TriangleFan => {
                    mesh::indexed(&vertices, &indices, &mut triangles)?
                }
                gltf::mesh::Mode::TriangleStrip => {
                    for (i, triangle) in indices.windows(3).enumerate() {
                        let triangle = if i % 2 == 0 {
                            [triangle[0], triangle[1], triangle[2]]
                        } else {
                            [triangle[1], triangle[0], triangle[2]]
                        };
                        mesh::indexed(&vertices, &triangle, &mut triangles)?;
                    }
                }
                _ => continue,
            }
        }
        if cached_triangles + triangles.len() > MAX_TRIANGLES {
            return Err(PreviewError::TooLarge);
        }
        cached_triangles += triangles.len();
        geometries.insert(m.index(), triangles);
    }
    fn visit(
        node: gltf::Node<'_>,
        parent: Mat4<f32>,
        geometries: &HashMap<usize, Vec<Triangle>>,
        out: &mut Vec<Triangle>,
        visits: &mut usize,
        active: &mut HashSet<usize>,
        depth: usize,
    ) -> Result<(), PreviewError> {
        *visits += 1;
        if depth > 64 || *visits > 100_000 {
            return Err(PreviewError::TooLarge);
        }
        if !active.insert(node.index()) {
            return Err(PreviewError::InvalidModel);
        }
        let world = parent * Mat4::from_col_arrays(node.transform().matrix());
        if let Some(m) = node.mesh() {
            append(
                out,
                geometries
                    .get(&m.index())
                    .ok_or(PreviewError::InvalidModel)?,
                world,
            )?;
        }
        for child in node.children() {
            visit(child, world, geometries, out, visits, active, depth + 1)?;
        }
        active.remove(&node.index());
        Ok(())
    }
    let mut out = Vec::new();
    let mut visits = 0;
    let mut active = HashSet::new();
    if let Some(scene) = gltf.default_scene().or_else(|| gltf.scenes().next()) {
        for node in scene.nodes() {
            visit(
                node,
                Mat4::identity(),
                &geometries,
                &mut out,
                &mut visits,
                &mut active,
                0,
            )?;
        }
    } else if gltf.nodes().len() > 0 {
        let children: HashSet<_> = gltf
            .nodes()
            .flat_map(|n| n.children().map(|c| c.index()).collect::<Vec<_>>())
            .collect();
        for node in gltf.nodes().filter(|n| !children.contains(&n.index())) {
            visit(
                node,
                Mat4::identity(),
                &geometries,
                &mut out,
                &mut visits,
                &mut active,
                0,
            )?;
        }
    } else {
        for triangles in geometries.values() {
            append(&mut out, triangles, Mat4::identity())?;
        }
    }
    y_up(&mut out);
    Ok(out)
}

pub(super) fn xml_document(bytes: &[u8]) -> Result<roxmltree::Document<'_>, PreviewError> {
    let text = std::str::from_utf8(bytes).map_err(|_| PreviewError::InvalidModel)?;
    if text.contains("<!ENTITY")
        || text.find("<!DOCTYPE").is_some_and(|p| {
            text[p..]
                .split('>')
                .next()
                .unwrap_or_default()
                .contains('[')
        })
    {
        return Err(PreviewError::InvalidModel);
    }
    roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: 2_000_000,
            entity_resolver: None,
        },
    )
    .map_err(|_| PreviewError::InvalidModel)
}

pub(super) fn numbers<T: std::str::FromStr>(text: &str) -> Result<Vec<T>, PreviewError> {
    let mut numbers = Vec::new();
    for value in text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
    {
        if numbers.len() >= MAX_VERTICES * 3 {
            return Err(PreviewError::TooLarge);
        }
        numbers.push(value.parse().map_err(|_| PreviewError::InvalidModel)?);
    }
    Ok(numbers)
}

pub(super) fn rotation(values: &[f32]) -> Result<Mat4<f32>, PreviewError> {
    if values.len() != 4 || !values.iter().all(|v| v.is_finite()) {
        return Err(PreviewError::InvalidModel);
    }
    let axis = Vec3::new(values[0], values[1], values[2]);
    Ok(if axis.magnitude_squared() < 1e-12 {
        Mat4::identity()
    } else {
        Mat4::rotation_3d(values[3], axis.normalized())
    })
}

pub(super) fn dae(bytes: &[u8]) -> Result<Vec<Triangle>, PreviewError> {
    let doc = xml_document(bytes)?;
    let root = doc.root_element();
    if !root.has_tag_name("COLLADA") {
        return Err(PreviewError::InvalidModel);
    }
    let ids: HashMap<_, _> = root
        .descendants()
        .filter_map(|node| node.attribute("id").map(|id| (id, node)))
        .collect();
    let reference = |uri: Option<&str>| {
        uri.and_then(|uri| uri.strip_prefix('#'))
            .and_then(|id| ids.get(id))
            .copied()
            .ok_or(PreviewError::InvalidModel)
    };
    let mut geometry = HashMap::new();
    let mut cached_triangles = 0usize;
    for g in root.descendants().filter(|n| n.has_tag_name("geometry")) {
        let Some(id) = g.attribute("id") else {
            continue;
        };
        let Some(m) = g.children().find(|n| n.has_tag_name("mesh")) else {
            continue;
        };
        let mut out = Vec::new();
        for primitive in m.children().filter(|n| {
            matches!(
                n.tag_name().name(),
                "triangles" | "polylist" | "polygons" | "trifans" | "tristrips"
            )
        }) {
            let inputs: Vec<_> = primitive
                .children()
                .filter(|n| n.has_tag_name("input"))
                .collect();
            let input = inputs
                .iter()
                .find(|n| matches!(n.attribute("semantic"), Some("VERTEX" | "POSITION")))
                .ok_or(PreviewError::InvalidModel)?;
            let source = reference(input.attribute("source"))?;
            let source = if source.has_tag_name("vertices") {
                reference(
                    source
                        .children()
                        .find(|n| n.attribute("semantic") == Some("POSITION"))
                        .and_then(|n| n.attribute("source")),
                )?
            } else {
                source
            };
            let array = source
                .children()
                .find(|n| n.has_tag_name("float_array"))
                .ok_or(PreviewError::InvalidModel)?;
            let values: Vec<f32> = numbers(array.text().unwrap_or_default())?;
            let accessor = source.descendants().find(|n| n.has_tag_name("accessor"));
            let stride: usize = accessor
                .and_then(|n| n.attribute("stride"))
                .unwrap_or("3")
                .parse()
                .map_err(|_| PreviewError::InvalidModel)?;
            let offset: usize = accessor
                .and_then(|n| n.attribute("offset"))
                .unwrap_or("0")
                .parse()
                .map_err(|_| PreviewError::InvalidModel)?;
            if stride < 3 || offset > values.len() {
                return Err(PreviewError::InvalidModel);
            }
            let vertices: Vec<Point> = values[offset..]
                .chunks_exact(stride)
                .map(|v| [v[0], v[1], v[2]])
                .collect();
            let vertex_offset: usize = input
                .attribute("offset")
                .unwrap_or("0")
                .parse()
                .map_err(|_| PreviewError::InvalidModel)?;
            let mut input_stride = 1;
            for input in &inputs {
                let offset: usize = input
                    .attribute("offset")
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| PreviewError::InvalidModel)?;
                if offset >= 64 {
                    return Err(PreviewError::Unsupported);
                }
                input_stride = input_stride.max(offset + 1);
            }
            for p in primitive.children().filter(|n| n.has_tag_name("p")) {
                let raw: Vec<usize> = numbers(p.text().unwrap_or_default())?;
                if !raw.len().is_multiple_of(input_stride) {
                    return Err(PreviewError::InvalidModel);
                }
                let indices: Vec<_> = raw
                    .chunks_exact(input_stride)
                    .map(|v| v[vertex_offset])
                    .collect();
                match primitive.tag_name().name() {
                    "triangles" => {
                        if !indices.len().is_multiple_of(3) {
                            return Err(PreviewError::InvalidModel);
                        }
                        for triangle in indices.as_chunks::<3>().0.iter() {
                            mesh::indexed(&vertices, triangle, &mut out)?;
                        }
                    }
                    "polylist" => {
                        let counts: Vec<usize> = numbers(
                            primitive
                                .children()
                                .find(|n| n.has_tag_name("vcount"))
                                .and_then(|n| n.text())
                                .unwrap_or_default(),
                        )?;
                        let mut offset: usize = 0;
                        for count in counts {
                            let end = offset.checked_add(count).ok_or(PreviewError::TooLarge)?;
                            mesh::indexed(
                                &vertices,
                                indices.get(offset..end).ok_or(PreviewError::InvalidModel)?,
                                &mut out,
                            )?;
                            offset = end;
                        }
                        if offset != indices.len() {
                            return Err(PreviewError::InvalidModel);
                        }
                    }
                    "tristrips" => {
                        for triangle in indices.windows(3) {
                            mesh::indexed(&vertices, triangle, &mut out)?;
                        }
                    }
                    _ => mesh::indexed(&vertices, &indices, &mut out)?,
                }
            }
        }
        if cached_triangles + out.len() > MAX_TRIANGLES {
            return Err(PreviewError::TooLarge);
        }
        cached_triangles += out.len();
        geometry.insert(id.to_owned(), out);
    }
    fn visit(
        node: roxmltree::Node<'_, '_>,
        parent: Mat4<f32>,
        ids: &HashMap<&str, roxmltree::Node<'_, '_>>,
        geometry: &HashMap<String, Vec<Triangle>>,
        out: &mut Vec<Triangle>,
        visits: &mut usize,
        depth: usize,
    ) -> Result<(), PreviewError> {
        *visits += 1;
        if depth > 64 || *visits > 100_000 {
            return Err(PreviewError::TooLarge);
        }
        let mut local = Mat4::identity();
        for child in node.children().filter(|n| n.is_element()) {
            let values: Vec<f32> = if matches!(
                child.tag_name().name(),
                "matrix" | "translate" | "scale" | "rotate"
            ) {
                numbers(child.text().unwrap_or_default())?
            } else {
                continue;
            };
            let step = match child.tag_name().name() {
                "matrix" if values.len() == 16 => Mat4::from_row_arrays(std::array::from_fn(|r| {
                    std::array::from_fn(|c| values[r * 4 + c])
                })),
                "translate" if values.len() == 3 => {
                    Mat4::translation_3d(Vec3::new(values[0], values[1], values[2]))
                }
                "scale" if values.len() == 3 => {
                    Mat4::scaling_3d(Vec3::new(values[0], values[1], values[2]))
                }
                "rotate" if values.len() == 4 => {
                    rotation(&[values[0], values[1], values[2], values[3].to_radians()])?
                }
                _ => return Err(PreviewError::InvalidModel),
            };
            local *= step;
        }
        let world = parent * local;
        for child in node.children().filter(|n| n.is_element()) {
            match child.tag_name().name() {
                "node" => visit(child, world, ids, geometry, out, visits, depth + 1)?,
                "instance_geometry" => {
                    let id = child
                        .attribute("url")
                        .and_then(|u| u.strip_prefix('#'))
                        .ok_or(PreviewError::InvalidModel)?;
                    append(
                        out,
                        geometry.get(id).ok_or(PreviewError::InvalidModel)?,
                        world,
                    )?;
                }
                "instance_node" => {
                    let id = child
                        .attribute("url")
                        .and_then(|u| u.strip_prefix('#'))
                        .ok_or(PreviewError::InvalidModel)?;
                    visit(
                        *ids.get(id).ok_or(PreviewError::InvalidModel)?,
                        world,
                        ids,
                        geometry,
                        out,
                        visits,
                        depth + 1,
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    let scene = root
        .children()
        .find(|n| n.has_tag_name("scene"))
        .and_then(|n| {
            n.children()
                .find(|n| n.has_tag_name("instance_visual_scene"))
        })
        .map(|n| reference(n.attribute("url")))
        .transpose()?
        .or_else(|| root.descendants().find(|n| n.has_tag_name("visual_scene")));
    let mut out = Vec::new();
    let mut visits = 0;
    if let Some(scene) = scene {
        visit(
            scene,
            Mat4::identity(),
            &ids,
            &geometry,
            &mut out,
            &mut visits,
            0,
        )?;
    } else {
        for triangles in geometry.values() {
            append(&mut out, triangles, Mat4::identity())?;
        }
    }
    match root
        .descendants()
        .find(|n| n.has_tag_name("up_axis"))
        .and_then(|n| n.text())
        .map(str::trim)
    {
        Some("Z_UP") => {}
        Some("X_UP") => {
            for point in out.iter_mut().flatten() {
                *point = [-point[2], point[1], point[0]];
            }
        }
        _ => y_up(&mut out),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_document_preserves_dtd_support_without_resolving_entities() {
        for xml in [
            "<X/>",
            "<!DOCTYPE X><X/>",
            "<!DOCTYPE X SYSTEM 'file:///missing-melt-preview.dtd'><X/>",
        ] {
            let doc = xml_document(xml.as_bytes()).unwrap();
            assert_eq!(doc.root_element().tag_name().name(), "X");
        }
        for xml in [
            "<!DOCTYPE X [<!ENTITY e 'a'>]><X>&e;</X>",
            "<!DOCTYPE X [<!ENTITY e SYSTEM 'file:///missing-melt-preview.txt'>]><X>&e;</X>",
            "<X>&unknown;</X>",
        ] {
            assert!(xml_document(xml.as_bytes()).is_err(), "{xml}");
        }
    }

    fn gltf_fixture(uri: String) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"uri": uri, "byteLength": 36}],
            "bufferViews": [{"buffer": 0, "byteLength": 36}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "nodes": [{"mesh":0,"translation":[5,0,0]}, {"mesh":0,"translation":[7,0,0]}],
            "scenes": [{"nodes": [0,1]}], "scene": 0
        })).unwrap()
    }

    fn vertex_bytes() -> Vec<u8> {
        [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect()
    }

    #[test]
    fn gltf_preserves_instances_and_supports_bounded_local_buffers() {
        let data = vertex_bytes();
        let json = gltf_fixture(format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(&data)
        ));
        let triangles = gltf(&json, None).unwrap();
        assert_eq!(triangles.len(), 2);
        assert_eq!(triangles[0][0][0], 5.);
        assert_eq!(triangles[1][0][0], 7.);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("part.bin"), &data).unwrap();
        let json = gltf_fixture("part.bin".to_owned());
        assert_eq!(
            gltf(&json, Some(&dir.path().join("part.gltf"))).unwrap(),
            triangles
        );
        assert!(gltf(&json, None).is_err());
        assert!(
            gltf(
                &gltf_fixture("../outside.bin".to_owned()),
                Some(&dir.path().join("part.gltf"))
            )
            .is_err()
        );
    }

    #[test]
    fn binary_glb_uses_the_same_geometry_pipeline() {
        let mut json: serde_json::Value =
            serde_json::from_slice(&gltf_fixture(String::new())).unwrap();
        json["buffers"][0].as_object_mut().unwrap().remove("uri");
        let mut json = serde_json::to_vec(&json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let data = vertex_bytes();
        let mut glb = b"glTF".to_vec();
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + 8 + data.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(data.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&data);
        let png = super::super::from_bytes("glb", &glb, (64, 36)).unwrap();
        assert!(super::super::validate_png(&png));
    }

    #[test]
    fn collada_preserves_repeated_instances_and_rejects_cycles() {
        let mesh = "<library_geometries><geometry id='g'><mesh><source id='p'><float_array>0 0 0 1 0 0 0 1 0</float_array><technique_common><accessor stride='3'/></technique_common></source><vertices id='v'><input semantic='POSITION' source='#p'/></vertices><triangles count='1'><input semantic='VERTEX' source='#v' offset='0'/><p>0 1 2</p></triangles></mesh></geometry></library_geometries>";
        let scene = "<library_visual_scenes><visual_scene id='s'><node><translate>5 0 0</translate><instance_geometry url='#g'/></node><node><translate>7 0 0</translate><instance_geometry url='#g'/></node></visual_scene></library_visual_scenes><scene><instance_visual_scene url='#s'/></scene>";
        let triangles = dae(format!("<COLLADA>{mesh}{scene}</COLLADA>").as_bytes()).unwrap();
        assert_eq!(triangles.len(), 2);
        assert_eq!(triangles[0][0][0], 5.);
        assert_eq!(triangles[1][0][0], 7.);
        let cycle = "<library_visual_scenes><visual_scene><node id='n'><instance_node url='#n'/></node></visual_scene></library_visual_scenes>";
        assert!(dae(format!("<COLLADA>{mesh}{cycle}</COLLADA>").as_bytes()).is_err());
        assert!(xml_document(b"<!DOCTYPE X [<!ENTITY e 'a'>]><X>&e;</X>").is_err());
    }
}
