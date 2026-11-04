use super::{Point, PreviewError};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Cursor, Read, Seek},
    path::Path,
};

const MAX_POINTS: usize = 1_000_000;
const MAX_RECORDS: u64 = 100_000_000;
const MAX_SCAN_BYTES: u64 = 512 << 20;
const MAX_PCD_BYTES: usize = 128 << 20;

/// Deterministic uniform thinning, retaining bounded storage even when a text
/// cloud has no count header. Recenter in f64 before converting survey data.
struct Sampler {
    points: Vec<Point>,
    origin: Option<[f64; 3]>,
    index: u64,
    stride: u64,
    limit: usize,
}
impl Sampler {
    fn new(limit: usize) -> Self {
        Self {
            points: Vec::new(),
            origin: None,
            index: 0,
            stride: 1,
            limit,
        }
    }
    fn add(&mut self, point: [f64; 3]) {
        let index = self.index;
        self.index += 1;
        if !point.iter().all(|v| v.is_finite()) || !index.is_multiple_of(self.stride) {
            return;
        }
        if self.points.len() >= self.limit {
            let mut i = 0;
            self.points.retain(|_| {
                let keep = i % 2 == 0;
                i += 1;
                keep
            });
            self.stride *= 2;
            if !index.is_multiple_of(self.stride) {
                return;
            }
        }
        let origin = *self.origin.get_or_insert(point);
        let p = std::array::from_fn(|i| (point[i] - origin[i]) as f32);
        if p.iter().all(|v| v.is_finite()) {
            self.points.push(p);
        }
    }
    fn finish(self) -> Result<Vec<Point>, PreviewError> {
        if self.points.is_empty() {
            Err(PreviewError::InvalidModel)
        } else {
            Ok(self.points)
        }
    }
}

fn text(reader: impl Read) -> Result<Vec<Point>, PreviewError> {
    let mut reader = BufReader::new(reader.take(MAX_SCAN_BYTES + 1));
    let mut sample = Sampler::new(MAX_POINTS);
    let mut consumed = 0u64;
    let mut line = Vec::new();
    for _ in 0..MAX_RECORDS {
        line.clear();
        let count = reader
            .by_ref()
            .take(65537)
            .read_until(b'\n', &mut line)
            .map_err(|_| PreviewError::InvalidModel)?;
        if count == 0 {
            return sample.finish();
        }
        consumed += count as u64;
        if count > 65536 || consumed > MAX_SCAN_BYTES {
            return Err(PreviewError::TooLarge);
        }
        let line = std::str::from_utf8(&line)
            .map_err(|_| PreviewError::InvalidModel)?
            .trim();
        if line.starts_with('#') {
            continue;
        }
        let mut values = line
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';'))
            .filter(|s| !s.is_empty());
        let point = std::array::from_fn(|_| values.next().and_then(|v| v.parse::<f64>().ok()));
        if let [Some(x), Some(y), Some(z)] = point {
            sample.add([x, y, z]);
        }
    }
    Err(PreviewError::TooLarge)
}

fn las(reader: impl Read + Seek + Send + Sync + 'static) -> Result<Vec<Point>, PreviewError> {
    let mut reader = las::Reader::new(reader).map_err(|_| PreviewError::InvalidModel)?;
    let count = reader.header().number_of_points();
    if count > MAX_RECORDS {
        return Err(PreviewError::TooLarge);
    }
    let mut remaining = count;
    let mut sample = Sampler::new(MAX_POINTS);
    let mut points = las::PointDataBuilder::new()
        .for_header(reader.header())
        .build();
    while remaining > 0 {
        let read = reader
            .fill_points(remaining.min(8192), &mut points)
            .map_err(|_| PreviewError::InvalidModel)?;
        if read == 0 {
            return Err(PreviewError::InvalidModel);
        }
        for point in points.points() {
            let point = point.map_err(|_| PreviewError::InvalidModel)?;
            sample.add([point.x, point.y, point.z]);
        }
        remaining -= read;
    }
    sample.finish()
}

fn e57(mut input: impl Read + Seek) -> Result<Vec<Point>, PreviewError> {
    let mut header = [0; 48];
    input
        .read_exact(&mut header)
        .map_err(|_| PreviewError::InvalidModel)?;
    let xml_length = u64::from_le_bytes(header[32..40].try_into().unwrap());
    let page = u64::from_le_bytes(header[40..48].try_into().unwrap());
    if xml_length > 8 << 20 || page != 1024 {
        return Err(PreviewError::TooLarge);
    }
    input.rewind().map_err(|_| PreviewError::InvalidModel)?;
    let mut reader = e57::E57Reader::new(input).map_err(|_| PreviewError::InvalidModel)?;
    let clouds = reader.pointclouds();
    let records = clouds
        .iter()
        .try_fold(0u64, |total, cloud| total.checked_add(cloud.records))
        .ok_or(PreviewError::TooLarge)?;
    if records > MAX_RECORDS {
        return Err(PreviewError::TooLarge);
    }
    let mut sample = Sampler::new(MAX_POINTS);
    for cloud in clouds {
        let mut points = reader
            .pointcloud_simple(&cloud)
            .map_err(|_| PreviewError::InvalidModel)?;
        points.apply_pose(true);
        points.spherical_to_cartesian(true);
        for point in points {
            if let e57::CartesianCoordinate::Valid { x, y, z } =
                point.map_err(|_| PreviewError::InvalidModel)?.cartesian
            {
                sample.add([x, y, z]);
            }
        }
    }
    sample.finish()
}

fn lzf(input: &[u8], length: usize) -> Result<Vec<u8>, PreviewError> {
    if length > MAX_PCD_BYTES {
        return Err(PreviewError::TooLarge);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let control = input[i] as usize;
        i += 1;
        if control < 32 {
            let count = control + 1;
            if out.len() + count > length {
                return Err(PreviewError::InvalidModel);
            }
            out.extend_from_slice(input.get(i..i + count).ok_or(PreviewError::InvalidModel)?);
            i += count;
        } else {
            let mut count = control >> 5;
            if count == 7 {
                count += *input.get(i).ok_or(PreviewError::InvalidModel)? as usize;
                i += 1;
            }
            count += 2;
            let distance = ((control & 31) << 8)
                + *input.get(i).ok_or(PreviewError::InvalidModel)? as usize
                + 1;
            i += 1;
            let start = out
                .len()
                .checked_sub(distance)
                .ok_or(PreviewError::InvalidModel)?;
            if out.len() + count > length {
                return Err(PreviewError::InvalidModel);
            }
            for k in 0..count {
                let byte = out[start + k];
                out.push(byte);
            }
        }
    }
    if out.len() != length {
        return Err(PreviewError::InvalidModel);
    }
    Ok(out)
}

fn pcd(bytes: &[u8]) -> Result<Vec<Point>, PreviewError> {
    let mut header: HashMap<String, Vec<String>> = HashMap::new();
    let mut start = 0;
    let mut body = None;
    for (i, byte) in bytes.iter().take(65536).enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line = std::str::from_utf8(&bytes[start..i]).map_err(|_| PreviewError::InvalidModel)?;
        start = i + 1;
        let mut words = line.split_whitespace();
        let key = words.next().unwrap_or_default().to_ascii_uppercase();
        if key.starts_with('#') {
            continue;
        }
        let values: Vec<_> = words.map(str::to_owned).collect();
        let data = key == "DATA";
        header.insert(key, values);
        if data {
            body = Some(start);
            break;
        }
    }
    let body = body.ok_or(PreviewError::InvalidModel)?;
    let fields = header.get("FIELDS").ok_or(PreviewError::InvalidModel)?;
    if fields.is_empty() || fields.len() > 128 {
        return Err(PreviewError::InvalidModel);
    }
    let parse = |key: &str, default: usize| -> Result<usize, PreviewError> {
        header
            .get(key)
            .and_then(|v| v.first())
            .map_or(Ok(default), |v| {
                v.parse().map_err(|_| PreviewError::InvalidModel)
            })
    };
    let count = parse(
        "POINTS",
        parse("WIDTH", 0)?
            .checked_mul(parse("HEIGHT", 1)?)
            .ok_or(PreviewError::TooLarge)?,
    )?;
    if count as u64 > MAX_RECORDS {
        return Err(PreviewError::TooLarge);
    }
    let parse_fields = |key: &str, default: usize| -> Result<Vec<usize>, PreviewError> {
        if let Some(values) = header.get(key) {
            if values.len() != fields.len() {
                return Err(PreviewError::InvalidModel);
            }
            values
                .iter()
                .map(|v| v.parse().map_err(|_| PreviewError::InvalidModel))
                .collect()
        } else {
            Ok(vec![default; fields.len()])
        }
    };
    let sizes = parse_fields("SIZE", 4)?;
    let counts = parse_fields("COUNT", 1)?;
    let types = header
        .get("TYPE")
        .cloned()
        .unwrap_or_else(|| vec!["F".to_owned(); fields.len()]);
    if types.len() != fields.len()
        || sizes.iter().any(|v| !matches!(v, 1 | 2 | 4 | 8))
        || counts.iter().any(|v| *v == 0 || *v > 1024)
    {
        return Err(PreviewError::InvalidModel);
    }
    let xyz: Vec<_> = ["x", "y", "z"]
        .iter()
        .map(|name| {
            fields
                .iter()
                .position(|f| f == name)
                .ok_or(PreviewError::Unsupported)
        })
        .collect::<Result<_, _>>()?;
    let mut offsets = Vec::new();
    let mut columns = Vec::new();
    let mut record = 0usize;
    let mut column = 0usize;
    for i in 0..fields.len() {
        offsets.push(record);
        columns.push(column);
        record += sizes[i] * counts[i];
        column += counts[i];
    }
    let mut sample = Sampler::new(MAX_POINTS);
    let mode = header
        .get("DATA")
        .and_then(|v| v.first())
        .ok_or(PreviewError::InvalidModel)?;
    if mode == "ascii" {
        let text = std::str::from_utf8(&bytes[body..]).map_err(|_| PreviewError::InvalidModel)?;
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        for _ in 0..count {
            let values: Vec<_> = lines
                .next()
                .ok_or(PreviewError::InvalidModel)?
                .split_whitespace()
                .collect();
            let mut point = [0.; 3];
            for (c, field) in xyz.iter().enumerate() {
                point[c] = values
                    .get(columns[*field])
                    .ok_or(PreviewError::InvalidModel)?
                    .parse()
                    .map_err(|_| PreviewError::InvalidModel)?;
            }
            sample.add(point);
        }
    } else {
        let expected = record.checked_mul(count).ok_or(PreviewError::TooLarge)?;
        if expected > MAX_PCD_BYTES {
            return Err(PreviewError::TooLarge);
        }
        let decoded;
        let raw = if mode == "binary_compressed" {
            let header = bytes
                .get(body..body + 8)
                .ok_or(PreviewError::InvalidModel)?;
            let compressed = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
            let uncompressed = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
            if uncompressed != expected {
                return Err(PreviewError::InvalidModel);
            }
            decoded = lzf(
                bytes
                    .get(body + 8..body + 8 + compressed)
                    .ok_or(PreviewError::InvalidModel)?,
                uncompressed,
            )?;
            decoded.as_slice()
        } else if mode == "binary" {
            bytes
                .get(body..body + expected)
                .ok_or(PreviewError::InvalidModel)?
        } else {
            return Err(PreviewError::Unsupported);
        };
        for i in 0..count {
            let mut point = [0.; 3];
            for (c, field) in xyz.iter().copied().enumerate() {
                let at = if mode == "binary_compressed" {
                    offsets[field] * count + i * sizes[field] * counts[field]
                } else {
                    i * record + offsets[field]
                };
                let data = raw
                    .get(at..at + sizes[field])
                    .ok_or(PreviewError::InvalidModel)?;
                point[c] = match (types[field].as_str(), sizes[field]) {
                    ("F", 4) => f32::from_le_bytes(data.try_into().unwrap()) as f64,
                    ("F", 8) => f64::from_le_bytes(data.try_into().unwrap()),
                    ("I", 1) => data[0] as i8 as f64,
                    ("U", 1) => data[0] as f64,
                    ("I", 2) => i16::from_le_bytes(data.try_into().unwrap()) as f64,
                    ("U", 2) => u16::from_le_bytes(data.try_into().unwrap()) as f64,
                    ("I", 4) => i32::from_le_bytes(data.try_into().unwrap()) as f64,
                    ("U", 4) => u32::from_le_bytes(data.try_into().unwrap()) as f64,
                    ("I", 8) => i64::from_le_bytes(data.try_into().unwrap()) as f64,
                    ("U", 8) => u64::from_le_bytes(data.try_into().unwrap()) as f64,
                    _ => return Err(PreviewError::Unsupported),
                };
            }
            sample.add(point);
        }
    }
    sample.finish()
}

pub(super) fn load_file(path: &Path, extension: &str) -> Result<Vec<Point>, PreviewError> {
    let file = std::fs::File::open(path).map_err(|_| PreviewError::Unavailable)?;
    if file
        .metadata()
        .map_err(|_| PreviewError::Unavailable)?
        .len()
        > MAX_SCAN_BYTES
    {
        return Err(PreviewError::TooLarge);
    }
    match extension {
        "xyz" | "pts" | "asc" => text(file),
        "las" => las(BufReader::new(file)),
        "e57" => e57(BufReader::new(file)),
        "pcd" => pcd(&super::read_bounded(file, MAX_PCD_BYTES)?),
        _ => Err(PreviewError::Unsupported),
    }
}

pub(super) fn load_bytes(extension: &str, bytes: &[u8]) -> Result<Vec<Point>, PreviewError> {
    match extension {
        "xyz" | "pts" | "asc" => text(Cursor::new(bytes)),
        "las" => las(Cursor::new(bytes.to_vec())),
        "e57" => e57(Cursor::new(bytes)),
        "pcd" => pcd(bytes),
        _ => Err(PreviewError::Unsupported),
    }
}

pub(super) struct PreparedPoints {
    pub(super) projected: Vec<Point>,
    pub(super) min: Point,
    pub(super) max: Point,
    pub(super) scale: f32,
    pub(super) ox: f32,
    pub(super) oy: f32,
}

pub(super) fn render(points: &[Point], size: (usize, usize)) -> Result<Vec<u8>, PreviewError> {
    let prepared = prepare(points, size)?;
    super::encode_png(&super::rendering::points(prepared, size), size)
}

pub(super) fn prepare(
    points: &[Point],
    (width, height): (usize, usize),
) -> Result<PreparedPoints, PreviewError> {
    let _span = super::profiling::span("prepare.points");
    if points.is_empty() {
        return Err(PreviewError::InvalidModel);
    }
    let projected: Vec<Point> = points
        .iter()
        .step_by(points.len().div_ceil(MAX_POINTS))
        .map(|[x, y, z]| {
            [
                (y - x) * 0.70710677,
                (x + y) * 0.4082483 - z * 0.8164966,
                (x + y + z) * 0.57735026,
            ]
        })
        .collect();
    if !projected.iter().flatten().all(|v| v.is_finite()) {
        return Err(PreviewError::InvalidModel);
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in &projected {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let sx = (max[0] - min[0]).max(0.001);
    let sy = (max[1] - min[1]).max(0.001);
    let scale = ((width as f32 * 0.86) / sx).min((height as f32 * 0.86) / sy);
    let ox = (width as f32 - sx * scale) / 2.;
    let oy = (height as f32 - sy * scale) / 2.;
    if !scale.is_finite() || !ox.is_finite() || !oy.is_finite() {
        return Err(PreviewError::InvalidModel);
    }
    Ok(PreparedPoints {
        projected,
        min,
        max,
        scale,
        ox,
        oy,
    })
}

pub(super) fn draw_cpu(prepared: &PreparedPoints, (width, height): (usize, usize)) -> Vec<u8> {
    let _span = super::profiling::span("draw.cpu.points");
    let PreparedPoints {
        projected,
        min,
        max,
        scale,
        ox,
        oy,
    } = prepared;
    let mut rgb = [10, 40, 58].repeat(width * height);
    let mut depth = vec![f32::NEG_INFINITY; width * height];
    let radius = (width / 640).clamp(1, 3) as i32;
    for p in projected {
        let x = ((p[0] - min[0]) * scale + ox) as i32;
        let y = ((p[1] - min[1]) * scale + oy) as i32;
        let shade = ((p[2] - min[2]) / (max[2] - min[2]).max(0.001)).clamp(0., 1.);
        for yy in (y - radius).max(0)..=(y + radius).min(height as i32 - 1) {
            for xx in (x - radius).max(0)..=(x + radius).min(width as i32 - 1) {
                let i = yy as usize * width + xx as usize;
                if p[2] > depth[i] {
                    depth[i] = p[2];
                    rgb[i * 3..i * 3 + 3].copy_from_slice(&[
                        (30. + shade * 35.) as u8,
                        (145. + shade * 80.) as u8,
                        (185. + shade * 55.) as u8,
                    ]);
                }
            }
        }
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scan and its meshed STL must not render as mirror images.
    #[test]
    fn projects_with_the_same_handedness_as_the_mesh_camera() {
        let view = vek::Mat4::<f32>::look_at_rh(
            vek::Vec3::new(2.0, 2.0, 2.0),
            vek::Vec3::zero(),
            vek::Vec3::new(0.0, 0.0, 1.0),
        );
        let cloud = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let prepared = prepare(&cloud, (64, 36)).unwrap();
        for (point, projected) in cloud.iter().zip(&prepared.projected) {
            let camera = view * vek::Vec4::from_point(vek::Vec3::from(*point));
            assert!(
                (camera.x - projected[0]).abs() < 1e-5,
                "{point:?}: camera x {} vs projected x {}",
                camera.x,
                projected[0]
            );
        }
    }

    #[test]
    fn samples_across_the_whole_cloud_and_preserves_survey_precision() {
        let mut sample = Sampler::new(8);
        for i in 0..64 {
            sample.add([1e9 + i as f64 * 0.01, 0., 0.]);
        }
        let points = sample.finish().unwrap();
        assert!(points.len() <= 8);
        assert!(points.last().unwrap()[0] > 0.5);
        assert!(points[1][0] > 0. && points[1][0] < 0.2);
        let points = text(Cursor::new(
            b"# scan\n3\n1000000000,0,0\n1000000000.01,1,0\n1000000000.02,0,1\n",
        ))
        .unwrap();
        assert!((points[1][0] - 0.01).abs() < 1e-5);
        let png = render(&points, (64, 36)).unwrap();
        assert!(super::super::validate_png(&png));
    }

    #[test]
    fn reads_ascii_binary_and_lzf_pcd_and_rejects_invalid_sizes() {
        let header = "VERSION .7\nFIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nCOUNT 1 1 1\nWIDTH 2\nHEIGHT 1\nPOINTS 2\n";
        let ascii = format!("{header}DATA ascii\n0 0 0\n1 2 3\n");
        let expected = pcd(ascii.as_bytes()).unwrap();
        let mut binary = format!("{header}DATA binary\n").into_bytes();
        for v in [0f32, 0., 0., 1., 2., 3.] {
            binary.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(pcd(&binary).unwrap(), expected);
        let raw = [0f32, 1., 0., 2., 0., 3.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let mut compressed = format!("{header}DATA binary_compressed\n").into_bytes();
        compressed.extend_from_slice(&25u32.to_le_bytes());
        compressed.extend_from_slice(&24u32.to_le_bytes());
        compressed.push(23);
        compressed.extend_from_slice(&raw);
        assert_eq!(pcd(&compressed).unwrap(), expected);
        assert!(lzf(&[255], 128).is_err());
        assert!(lzf(&[], MAX_PCD_BYTES + 1).is_err());
        assert_eq!(lzf(&[0, b'a', 96, 0], 6).unwrap(), b"aaaaaa");
        assert_eq!(lzf(&[0, b'a', 224, 10, 0], 20).unwrap(), vec![b'a'; 20]);
    }

    #[test]
    fn reads_las_and_e57_scans_with_poses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.las");
        {
            let mut writer = las::Writer::from_path(&path, las::Header::default()).unwrap();
            for x in [0., 1., 2.] {
                writer
                    .write_point(las::Point {
                        x,
                        y: x,
                        z: 0.,
                        ..las::Point::default()
                    })
                    .unwrap();
            }
            writer.close().unwrap();
        }
        assert_eq!(load_file(&path, "las").unwrap().len(), 3);

        let path = dir.path().join("sample.e57");
        let mut writer = e57::E57Writer::from_file(&path, "sample-file").unwrap();
        for (guid, x) in [("first", 0.), ("second", 10.)] {
            let prototype = [
                e57::RecordName::CartesianX,
                e57::RecordName::CartesianY,
                e57::RecordName::CartesianZ,
            ]
            .into_iter()
            .map(|name| e57::Record {
                name,
                data_type: e57::RecordDataType::Double {
                    min: None,
                    max: None,
                },
            })
            .collect();
            let mut cloud = writer.add_pointcloud(guid, prototype).unwrap();
            cloud.set_transform(Some(e57::Transform {
                translation: e57::Translation { x, y: 0., z: 0. },
                ..e57::Transform::default()
            }));
            for value in [0., 1.] {
                cloud
                    .add_point(vec![
                        e57::RecordValue::Double(value),
                        e57::RecordValue::Double(0.),
                        e57::RecordValue::Double(0.),
                    ])
                    .unwrap();
            }
            cloud.finalize().unwrap();
        }
        writer.finalize().unwrap();
        drop(writer);
        let points = load_file(&path, "e57").unwrap();
        assert_eq!(points.len(), 4);
        assert_eq!(points[2][0], 10.);
        assert_eq!(points[3][0], 11.);
    }

    #[test]
    fn renders_faceless_ply_and_rejects_oversized_scan_files() {
        let ply = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty double x\nproperty double y\nproperty double z\nend_header\n0 0 0\n1 0 0\n0 1 0\n";
        let png = super::super::from_bytes("ply", ply, (64, 36)).unwrap();
        let image = image::load_from_memory(&png).unwrap().to_rgb8();
        assert!(image.pixels().any(|p| p.0 != [10, 40, 58]));
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(MAX_SCAN_BYTES + 1).unwrap();
        assert!(matches!(
            load_file(file.path(), "xyz"),
            Err(PreviewError::TooLarge)
        ));
    }
}
