use std::{
    collections::HashMap,
    io::{Cursor, Read, Seek},
};

use euc::{DepthStrategy, Pipeline, buffer::Buffer2d, rasterizer};
use vek::{Mat4, Vec3, Vec4};

type Point = [f32; 3];
type Triangle = [Point; 3];

/// Render sizes. Both are 16:9, so the perspective projection frames a model
/// identically at either one and the large render is the grid thumbnail with
/// more pixels rather than a different picture.
pub const GRID_SIZE: (usize, usize) = (640, 360);
pub const LARGE_SIZE: (usize, usize) = (1920, 1080);
pub const MAX_PREVIEW_BYTES: usize = 16 << 20;
pub const MAX_MODEL_BYTES: usize = 32 << 20;
const MAX_PREVIEW_DIMENSION: u32 = 4096;
const MAX_PREVIEW_PIXELS: u64 = 16_000_000;

#[derive(Debug)]
pub enum PreviewError {
    InvalidModel,
    TooLarge,
    Png,
}

/// Returns the first supported model stored in a ZIP, preserving archive
/// order. Entries are read in-place rather than extracted to disk, and the
/// decompressed size is bounded before allocation to avoid ZIP bombs.
pub fn first_model_in_zip(reader: impl Read + Seek) -> Result<(String, Vec<u8>), PreviewError> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|_| PreviewError::InvalidModel)?;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| PreviewError::InvalidModel)?;
        if entry.is_dir() {
            continue;
        }
        let extension = entry
            .name()
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "3mf" | "stl" | "obj") {
            continue;
        }
        if entry.size() > MAX_MODEL_BYTES as u64 {
            return Err(PreviewError::TooLarge);
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .take(MAX_MODEL_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PreviewError::InvalidModel)?;
        if bytes.len() > MAX_MODEL_BYTES {
            return Err(PreviewError::TooLarge);
        }
        return Ok((extension, bytes));
    }
    Err(PreviewError::InvalidModel)
}

pub fn validate_png(bytes: &[u8]) -> bool {
    if bytes.len() < 24
        || bytes.len() > MAX_PREVIEW_BYTES
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[12..16] != b"IHDR"
    {
        return false;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    width > 0
        && height > 0
        && width <= MAX_PREVIEW_DIMENSION
        && height <= MAX_PREVIEW_DIMENSION
        && u64::from(width) * u64::from(height) <= MAX_PREVIEW_PIXELS
}

pub fn rasterize(
    extension: &str,
    bytes: &[u8],
    size: (usize, usize),
) -> Result<Vec<u8>, PreviewError> {
    let triangles = match extension {
        "obj" => parse_obj(bytes),
        "stl" => parse_stl(bytes),
        _ => Vec::new(),
    };
    if triangles.is_empty() {
        return Err(PreviewError::InvalidModel);
    }
    encode_png(&render(&triangles, size), size)
}

pub fn rasterize_3mf(bytes: &[u8], size: (usize, usize)) -> Result<Vec<u8>, PreviewError> {
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
    encode_png(&render(&triangles, size), size)
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

struct MeshVertex {
    position: Vec3<f32>,
    normal: Vec3<f32>,
}

struct MeshPipeline {
    mvp: Mat4<f32>,
    light_dir: Vec3<f32>,
}

impl Pipeline for MeshPipeline {
    type Vertex = MeshVertex;
    type VsOut = Vec3<f32>;
    type Pixel = [u8; 3];

    #[inline(always)]
    fn vert(&self, vertex: &Self::Vertex) -> ([f32; 4], Self::VsOut) {
        let clip = self.mvp * Vec4::from_point(vertex.position);
        (clip.into_array(), vertex.normal)
    }

    #[inline(always)]
    fn get_depth_strategy(&self) -> DepthStrategy {
        DepthStrategy::IfLessWrite
    }

    #[inline(always)]
    fn frag(&self, normal: &Vec3<f32>) -> Self::Pixel {
        // Interpolating vertex normals and evaluating the light here gives
        // curved meshes smooth (Gouraud/Phong-style) shading. Computing a
        // single light value per face makes every source triangle visible.
        let diffuse = normal
            .try_normalized()
            .map_or(0.0, |normal| normal.dot(self.light_dir).abs());
        [
            (30.0 + diffuse * 35.0).clamp(0.0, 255.0) as u8,
            (145.0 + diffuse * 80.0).clamp(0.0, 255.0) as u8,
            (185.0 + diffuse * 55.0).clamp(0.0, 255.0) as u8,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PointKey([u32; 3]);

impl From<Vec3<f32>> for PointKey {
    fn from(point: Vec3<f32>) -> Self {
        // STL files commonly mix -0 and +0 at otherwise shared vertices.
        // Canonicalizing zero lets those faces participate in the same
        // averaged normal without fuzzy position matching.
        Self([point.x, point.y, point.z].map(
            |value| {
                if value == 0.0 { 0 } else { value.to_bits() }
            },
        ))
    }
}

#[derive(Debug)]
struct NormalCluster {
    sum: Vec3<f32>,
}

const SMOOTH_ANGLE_COSINE: f32 = 0.5; // 60 degrees

fn face_normal(triangle: &Triangle) -> Option<Vec3<f32>> {
    let points = (*triangle).map(Vec3::from);
    (points[1] - points[0])
        .cross(points[2] - points[0])
        .try_normalized()
}

/// Build a small set of normal clusters at each shared position. Faces less
/// than 60 degrees apart share a smooth normal, while deliberate corners
/// (box edges, chamfers, and similar features) remain crisp.
fn normal_clusters(triangles: &[Triangle]) -> HashMap<PointKey, Vec<NormalCluster>> {
    let mut normals: HashMap<PointKey, Vec<NormalCluster>> = HashMap::new();
    for triangle in triangles {
        let Some(face_normal) = face_normal(triangle) else {
            continue;
        };
        for point in (*triangle).map(Vec3::from) {
            let clusters = normals.entry(point.into()).or_default();
            if let Some(cluster) = clusters.iter_mut().find(|cluster| {
                cluster
                    .sum
                    .try_normalized()
                    .is_some_and(|normal| normal.dot(face_normal) >= SMOOTH_ANGLE_COSINE)
            }) {
                cluster.sum += face_normal;
            } else {
                clusters.push(NormalCluster { sum: face_normal });
            }
        }
    }
    normals
}

fn smooth_normal(
    clusters: &HashMap<PointKey, Vec<NormalCluster>>,
    point: Vec3<f32>,
    face_normal: Vec3<f32>,
) -> Vec3<f32> {
    clusters
        .get(&point.into())
        .and_then(|clusters| {
            clusters
                .iter()
                .filter_map(|cluster| cluster.sum.try_normalized())
                .max_by(|left, right| left.dot(face_normal).total_cmp(&right.dot(face_normal)))
        })
        .filter(|normal| normal.dot(face_normal) >= SMOOTH_ANGLE_COSINE)
        .unwrap_or(face_normal)
}

/// Renders triangles with `euc`, a real (tested) software triangle
/// rasterizer, instead of a hand-rolled scanline fill — the previous
/// hand-rolled bounding-box computation had a correctness bug that silently
/// turned every render into a near full-canvas fill and pegged a CPU core
/// on real meshes.
fn render(triangles: &[Triangle], (width, height): (usize, usize)) -> Vec<u8> {
    let normals = normal_clusters(triangles);
    let mut vertices = Vec::with_capacity(triangles.len() * 3);
    let mut min = Vec3::broadcast(f32::INFINITY);
    let mut max = Vec3::broadcast(f32::NEG_INFINITY);
    for triangle in triangles {
        let points = (*triangle).map(Vec3::from);
        let Some(face_normal) = face_normal(triangle) else {
            continue;
        };
        for point in points {
            min = Vec3::partial_min(min, point);
            max = Vec3::partial_max(max, point);
            vertices.push(MeshVertex {
                position: point,
                normal: smooth_normal(&normals, point, face_normal),
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
        width as f32,
        height as f32,
        1.0,
        10.0,
    );
    let pipeline = MeshPipeline {
        mvp: proj * view * model,
        light_dir: Vec3::new(-0.4, 0.8, 0.45).normalized(),
    };

    let mut color = Buffer2d::new([width, height], [10u8, 40, 58]);
    let mut depth = Buffer2d::new([width, height], 1.0f32);
    pipeline.draw::<rasterizer::Triangles<_, rasterizer::BackfaceCullingDisabled>, _>(
        &vertices,
        &mut color,
        Some(&mut depth),
    );

    color.as_ref().iter().flatten().copied().collect()
}

fn encode_png(rgb: &[u8], (width, height): (usize, usize)) -> Result<Vec<u8>, PreviewError> {
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width as u32, height as u32);
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
        let png = rasterize("stl", &binary_stl_with_triangles(1), GRID_SIZE).unwrap();
        assert!(!png.is_empty());
    }

    #[test]
    fn rasterizes_an_obj() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let png = rasterize("obj", obj.as_bytes(), GRID_SIZE).unwrap();
        assert!(!png.is_empty());
    }

    /// The preview dialog shows the render several times the size of the grid
    /// thumbnail, so the requested size has to reach the PNG rather than being
    /// silently ignored by a leftover constant.
    #[test]
    fn rasterizes_at_the_requested_size() {
        let png = rasterize("stl", &binary_stl_with_triangles(1), LARGE_SIZE).unwrap();
        assert!(validate_png(&png));
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            LARGE_SIZE.0 as u32
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
            LARGE_SIZE.1 as u32
        );
    }

    #[test]
    fn averages_normals_across_a_smooth_shared_edge() {
        let triangles = [
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 1.0]],
        ];
        let clusters = normal_clusters(&triangles);
        let first_face = face_normal(&triangles[0]).unwrap();
        let normal = smooth_normal(&clusters, Vec3::zero(), first_face);

        assert!(
            normal.y < 0.0,
            "the adjacent face must influence the normal"
        );
        assert!(normal.z > 0.0);
        assert_ne!(normal, first_face);
    }

    #[test]
    fn keeps_ninety_degree_edges_sharp() {
        let triangles = [
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        ];
        let clusters = normal_clusters(&triangles);
        let first_face = face_normal(&triangles[0]).unwrap();

        assert_eq!(
            smooth_normal(&clusters, Vec3::zero(), first_face),
            first_face
        );
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
        let png = rasterize_3mf(&buffer, GRID_SIZE).unwrap();
        assert!(!png.is_empty());
    }

    #[test]
    fn selects_only_the_first_supported_model_in_a_zip() {
        let first = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let second = binary_stl_with_triangles(1);
        let mut buffer = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            zip.start_file("notes/readme.txt", options).unwrap();
            zip.write_all(b"not a model").unwrap();
            zip.start_file("models/first.OBJ", options).unwrap();
            zip.write_all(first).unwrap();
            zip.start_file("models/second.stl", options).unwrap();
            zip.write_all(&second).unwrap();
            zip.finish().unwrap();
        }

        let (extension, bytes) = first_model_in_zip(Cursor::new(buffer)).unwrap();
        assert_eq!(extension, "obj");
        assert_eq!(bytes, first);
    }

    #[test]
    fn rejects_a_zip_without_a_supported_model() {
        let mut buffer = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            zip.start_file("readme.txt", options).unwrap();
            zip.write_all(b"not a model").unwrap();
            zip.finish().unwrap();
        }

        assert!(matches!(
            first_model_in_zip(Cursor::new(buffer)),
            Err(PreviewError::InvalidModel)
        ));
    }

    /// A real high-poly print (hundreds of thousands of triangles) must
    /// remain bounded enough for the two-worker preview queue. Every triangle
    /// is deliberately rendered: striding through this list tears holes in
    /// the surface because mesh triangles are topology, not independent
    /// samples.
    #[test]
    fn renders_a_high_poly_mesh_without_dropping_faces() {
        let bytes = binary_stl_with_triangles(500_000);
        let start = Instant::now();
        let png = rasterize("stl", &bytes, GRID_SIZE).unwrap();
        assert!(!png.is_empty());
        assert!(
            start.elapsed().as_secs() < 8,
            "rendering took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn validates_png_signature_and_bounded_dimensions() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640_u32.to_be_bytes());
        png.extend_from_slice(&360_u32.to_be_bytes());
        assert!(validate_png(&png));

        png[16..20].copy_from_slice(&5000_u32.to_be_bytes());
        assert!(!validate_png(&png));
        assert!(!validate_png(b"not a png"));
    }
}
