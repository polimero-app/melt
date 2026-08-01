use std::io::{Cursor, Read};

use euc::{DepthStrategy, Pipeline, buffer::Buffer2d, rasterizer};
use vek::{Mat4, Vec3, Vec4};

type Point = [f32; 3];
type Triangle = [Point; 3];

const WIDTH: usize = 640;
const HEIGHT: usize = 360;

#[derive(Debug)]
pub enum PreviewError {
    InvalidModel,
    Png,
}

pub fn rasterize(extension: &str, bytes: &[u8]) -> Result<Vec<u8>, PreviewError> {
    let triangles = match extension {
        "obj" => parse_obj(bytes),
        "stl" => parse_stl(bytes),
        _ => Vec::new(),
    };
    if triangles.is_empty() {
        return Err(PreviewError::InvalidModel);
    }
    encode_png(&render(&triangles))
}

pub fn rasterize_3mf(bytes: &[u8]) -> Result<Vec<u8>, PreviewError> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| PreviewError::InvalidModel)?;
    let mut model = archive
        .by_name("3D/3dmodel.model")
        .map_err(|_| PreviewError::InvalidModel)?;
    let mut source = String::new();
    model
        .read_to_string(&mut source)
        .map_err(|_| PreviewError::InvalidModel)?;
    let mut triangles = Vec::new();
    for mesh in source.split("<mesh").skip(1) {
        let section = mesh.split("</mesh>").next().unwrap_or(mesh);
        let vertices: Vec<Point> = section
            .split("<vertex")
            .skip(1)
            .filter_map(|tag| {
                Some([
                    xml_attribute(tag, "x")?.parse().ok()?,
                    xml_attribute(tag, "y")?.parse().ok()?,
                    xml_attribute(tag, "z")?.parse().ok()?,
                ])
            })
            .collect();
        for tag in section.split("<triangle").skip(1) {
            let indexes = [
                xml_attribute(tag, "v1"),
                xml_attribute(tag, "v2"),
                xml_attribute(tag, "v3"),
            ];
            let [Some(a), Some(b), Some(c)] = indexes.map(|index| index?.parse::<usize>().ok())
            else {
                continue;
            };
            if let (Some(&a), Some(&b), Some(&c)) =
                (vertices.get(a), vertices.get(b), vertices.get(c))
            {
                triangles.push([a, b, c]);
            }
        }
    }
    if triangles.is_empty() {
        return Err(PreviewError::InvalidModel);
    }
    encode_png(&render(&triangles))
}

fn xml_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("{name}=\"");
    let start = tag.find(&marker)? + marker.len();
    Some(&tag[start..tag[start..].find('"')? + start])
}

fn parse_obj(bytes: &[u8]) -> Vec<Triangle> {
    let source = String::from_utf8_lossy(bytes);
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for line in source.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let point = [
                    parts.next().and_then(|value| value.parse().ok()),
                    parts.next().and_then(|value| value.parse().ok()),
                    parts.next().and_then(|value| value.parse().ok()),
                ];
                if let [Some(x), Some(y), Some(z)] = point {
                    vertices.push([x, y, z]);
                }
            }
            Some("f") => {
                let indices: Vec<_> = parts
                    .filter_map(|part| part.split('/').next())
                    .filter_map(|value| value.parse::<isize>().ok())
                    .filter_map(|index| {
                        let index = if index < 0 {
                            vertices.len() as isize + index
                        } else {
                            index - 1
                        };
                        (index >= 0).then_some(index as usize)
                    })
                    .collect();
                for index in 1..indices.len().saturating_sub(1) {
                    if let (Some(&a), Some(&b), Some(&c)) = (
                        vertices.get(indices[0]),
                        vertices.get(indices[index]),
                        vertices.get(indices[index + 1]),
                    ) {
                        triangles.push([a, b, c]);
                    }
                }
            }
            _ => {}
        }
    }
    triangles
}

fn parse_stl(bytes: &[u8]) -> Vec<Triangle> {
    if bytes.len() >= 84 {
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        if count > 0 && 84usize.saturating_add(count.saturating_mul(50)) <= bytes.len() {
            let mut triangles = Vec::with_capacity(count);
            for triangle in 0..count {
                let offset = 84 + triangle * 50 + 12;
                let mut points = [[0.0; 3]; 3];
                for (vertex, point) in points.iter_mut().enumerate() {
                    let start = offset + vertex * 12;
                    for (axis, coordinate) in point.iter_mut().enumerate() {
                        *coordinate = f32::from_le_bytes(
                            bytes[start + axis * 4..start + axis * 4 + 4]
                                .try_into()
                                .unwrap(),
                        );
                    }
                }
                if points.iter().flatten().all(|value| value.is_finite()) {
                    triangles.push(points);
                }
            }
            if !triangles.is_empty() {
                return triangles;
            }
        }
    }

    let mut points = Vec::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        let mut parts = line.split_whitespace();
        if parts
            .next()
            .is_some_and(|value| value.eq_ignore_ascii_case("vertex"))
        {
            let point = [
                parts.next().and_then(|value| value.parse().ok()),
                parts.next().and_then(|value| value.parse().ok()),
                parts.next().and_then(|value| value.parse().ok()),
            ];
            if let [Some(x), Some(y), Some(z)] = point {
                points.push([x, y, z]);
            }
        }
    }
    points
        .chunks_exact(3)
        .map(|chunk| [chunk[0], chunk[1], chunk[2]])
        .collect()
}

/// Thumbnails don't need full mesh fidelity, and the rasterizer is
/// O(triangles) per draw call; without a cap, a real high-poly print
/// (hundreds of thousands of triangles, common for detailed models) can
/// still take a while to render. Striding down to this many triangles keeps
/// preview generation fast regardless of source mesh complexity.
const MAX_RENDER_TRIANGLES: usize = 12_000;

struct MeshVertex {
    position: Vec3<f32>,
    light: f32,
}

struct MeshPipeline {
    mvp: Mat4<f32>,
}

impl Pipeline for MeshPipeline {
    type Vertex = MeshVertex;
    type VsOut = f32;
    type Pixel = [u8; 3];

    #[inline(always)]
    fn vert(&self, vertex: &Self::Vertex) -> ([f32; 4], Self::VsOut) {
        let clip = self.mvp * Vec4::from_point(vertex.position);
        (clip.into_array(), vertex.light)
    }

    #[inline(always)]
    fn get_depth_strategy(&self) -> DepthStrategy {
        DepthStrategy::IfLessWrite
    }

    #[inline(always)]
    fn frag(&self, light: &f32) -> Self::Pixel {
        [
            (30.0 + light * 35.0).clamp(0.0, 255.0) as u8,
            (145.0 + light * 80.0).clamp(0.0, 255.0) as u8,
            (185.0 + light * 55.0).clamp(0.0, 255.0) as u8,
        ]
    }
}

/// Renders triangles with `euc`, a real (tested) software triangle
/// rasterizer, instead of a hand-rolled scanline fill — the previous
/// hand-rolled bounding-box computation had a correctness bug that silently
/// turned every render into a near full-canvas fill and pegged a CPU core
/// on real meshes.
fn render(triangles: &[Triangle]) -> Vec<u8> {
    let stride = (triangles.len() / MAX_RENDER_TRIANGLES).max(1);
    let sampled = triangles.iter().step_by(stride);

    // Flat (per-face) shading: every vertex of a triangle carries that
    // triangle's own face-normal lighting, matching the look of the
    // previous renderer without needing vertex-normal averaging.
    let light_dir = Vec3::new(-0.4, 0.8, 0.45);
    let mut vertices = Vec::with_capacity(triangles.len().min(MAX_RENDER_TRIANGLES) * 3);
    let mut min = Vec3::broadcast(f32::INFINITY);
    let mut max = Vec3::broadcast(f32::NEG_INFINITY);
    for triangle in sampled {
        let points = (*triangle).map(Vec3::from);
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        let light = (normal.dot(light_dir) / normal.magnitude().max(0.001)).abs();
        for point in points {
            min = Vec3::partial_min(min, point);
            max = Vec3::partial_max(max, point);
            vertices.push(MeshVertex {
                position: point,
                light,
            });
        }
    }

    let center = (min + max) * 0.5;
    let extent = (max - min).reduce_partial_max().max(0.001);
    let model =
        Mat4::<f32>::scaling_3d(Vec3::broadcast(2.0 / extent)) * Mat4::translation_3d(-center);
    let view = Mat4::<f32>::look_at_rh(
        Vec3::new(2.0, 2.0, 2.0),
        Vec3::zero(),
        Vec3::new(0.0, 0.0, 1.0),
    );
    let proj = Mat4::<f32>::perspective_fov_rh_no(
        30f32.to_radians(),
        WIDTH as f32,
        HEIGHT as f32,
        1.0,
        10.0,
    );
    let pipeline = MeshPipeline {
        mvp: proj * view * model,
    };

    let mut color = Buffer2d::new([WIDTH, HEIGHT], [10u8, 40, 58]);
    let mut depth = Buffer2d::new([WIDTH, HEIGHT], 1.0f32);
    pipeline.draw::<rasterizer::Triangles<_, rasterizer::BackfaceCullingDisabled>, _>(
        &vertices,
        &mut color,
        Some(&mut depth),
    );

    color.as_ref().iter().flatten().copied().collect()
}

fn encode_png(rgb: &[u8]) -> Result<Vec<u8>, PreviewError> {
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, WIDTH as u32, HEIGHT as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|_| PreviewError::Png)?;
    writer
        .write_image_data(rgb)
        .map_err(|_| PreviewError::Png)?;
    drop(writer);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, time::Instant};

    fn binary_stl_with_triangles(count: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&count.to_le_bytes());
        for index in 0..count {
            let offset = index as f32 * 0.001;
            bytes.extend_from_slice(&[0.0f32; 3].map(f32::to_le_bytes).concat()); // normal
            bytes.extend_from_slice(&[offset, 0.0, 0.0].map(f32::to_le_bytes).concat());
            bytes.extend_from_slice(&[offset + 1.0, 0.0, 0.0].map(f32::to_le_bytes).concat());
            bytes.extend_from_slice(&[offset, 1.0, 0.0].map(f32::to_le_bytes).concat());
            bytes.extend_from_slice(&[0u8, 0]); // attribute byte count
        }
        bytes
    }

    #[test]
    fn rasterizes_a_small_binary_stl() {
        let png = rasterize("stl", &binary_stl_with_triangles(1)).unwrap();
        assert!(!png.is_empty());
    }

    #[test]
    fn rasterizes_an_obj() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let png = rasterize("obj", obj.as_bytes()).unwrap();
        assert!(!png.is_empty());
    }

    #[test]
    fn rasterizes_a_3mf_without_an_embedded_thumbnail() {
        let model_xml = r#"<model><resources><object id="1"><mesh>
            <vertices>
                <vertex x="0" y="0" z="0"/>
                <vertex x="1" y="0" z="0"/>
                <vertex x="0" y="1" z="0"/>
            </vertices>
            <triangles><triangle v1="0" v2="1" v3="2"/></triangles>
        </mesh></object></resources></model>"#;
        let mut buffer = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            zip.start_file("3D/3dmodel.model", options).unwrap();
            zip.write_all(model_xml.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let png = rasterize_3mf(&buffer).unwrap();
        assert!(!png.is_empty());
    }

    /// A real high-poly print (hundreds of thousands of triangles) must
    /// still render in well under a second; without the `MAX_RENDER_TRIANGLES`
    /// downsample this took multiple seconds per thumbnail and pegged a CPU
    /// core when a folder had many such models open at once.
    #[test]
    fn caps_render_cost_for_high_poly_meshes() {
        let bytes = binary_stl_with_triangles(500_000);
        let start = Instant::now();
        let png = rasterize("stl", &bytes).unwrap();
        assert!(!png.is_empty());
        assert!(
            start.elapsed().as_secs() < 2,
            "rendering took {:?}, the triangle cap isn't limiting cost",
            start.elapsed()
        );
    }
}
