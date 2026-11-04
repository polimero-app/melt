//! Reproducible, uncached end-to-end preview timings using synthetic geometry.
use melt_desktop::preview;
use std::{path::PathBuf, time::Instant};

fn stl(triangles: &[[[f32; 3]; 3]]) -> Vec<u8> {
    let mut bytes = vec![0; 80];
    bytes.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    for triangle in triangles {
        bytes.extend_from_slice(&[0; 12]);
        for point in triangle {
            for value in point {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&[0; 2]);
    }
    bytes
}

fn cube() -> Vec<u8> {
    let vertices = [
        [-1., -1., -1.],
        [1., -1., -1.],
        [1., 1., -1.],
        [-1., 1., -1.],
        [-1., -1., 1.],
        [1., -1., 1.],
        [1., 1., 1.],
        [-1., 1., 1.],
    ];
    let faces = [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [1, 5, 6, 2],
        [2, 6, 7, 3],
        [3, 7, 4, 0],
    ];
    let mut triangles = Vec::new();
    for [a, b, c, d] in faces {
        triangles.push([vertices[a], vertices[b], vertices[c]]);
        triangles.push([vertices[a], vertices[c], vertices[d]]);
    }
    stl(&triangles)
}

fn sphere() -> Vec<u8> {
    let (rows, cols) = (128, 256);
    let vertex = |row: usize, col: usize| {
        let theta = row as f32 * std::f32::consts::PI / rows as f32;
        let phi = col as f32 * std::f32::consts::TAU / cols as f32;
        [
            100. * theta.sin() * phi.cos(),
            100. * theta.sin() * phi.sin(),
            100. * theta.cos(),
        ]
    };
    let mut triangles = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let [a, b, c, d] = [
                vertex(row, col),
                vertex(row + 1, col),
                vertex(row + 1, col + 1),
                vertex(row, col + 1),
            ];
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
    }
    stl(&triangles)
}

fn cloud() -> Vec<u8> {
    let mut data = b"VERSION .7\nFIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nCOUNT 1 1 1\nWIDTH 1000000\nHEIGHT 1\nPOINTS 1000000\nDATA binary\n".to_vec();
    for z in 0..100 {
        for y in 0..100 {
            for x in 0..100 {
                for value in [x as f32, y as f32, z as f32] {
                    data.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
    data
}

fn run() -> Result<(), String> {
    let mut output = PathBuf::from("/tmp/melt-preview-bench");
    let mut iterations = 3usize;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => output = args.next().ok_or("missing output directory")?.into(),
            "--iterations" => {
                iterations = args
                    .next()
                    .ok_or("missing iteration count")?
                    .parse()
                    .map_err(|_| "invalid iteration count")?
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    if !(1..=50).contains(&iterations) {
        return Err("iterations must be 1–50".to_owned());
    }
    std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    for (name, extension, bytes, items) in [
        ("small-mesh", "stl", cube(), 12),
        ("dense-mesh", "stl", sphere(), 65_536),
        ("point-cloud", "pcd", cloud(), 1_000_000),
    ] {
        for (label, size) in [("grid", preview::GRID_SIZE), ("large", preview::LARGE_SIZE)] {
            let mut samples = Vec::new();
            for iteration in 0..iterations {
                let start = Instant::now();
                let (png, timings) =
                    preview::profiling::capture(|| preview::from_bytes(extension, &bytes, size));
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
                let png = png.map_err(|error| format!("{name}: {error:?}"))?;
                if iteration == 0 {
                    std::fs::write(output.join(format!("{name}-{label}.png")), &png)
                        .map_err(|error| error.to_string())?;
                }
                samples.push(serde_json::json!({"elapsedMs":elapsed_ms,"pngBytes":png.len(),"stages":timings}));
            }
            results.push(serde_json::json!({"case":name,"size":size,"items":items,"inputBytes":bytes.len(),"samples":samples}));
            eprintln!("benchmarked {name} {label}");
        }
    }
    let report = serde_json::json!({"schemaVersion":1,"renderer":"cpu","profile":if cfg!(debug_assertions) {"debug"} else {"release"},"results":results});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .map_err(|error| error.to_string())?;
    println!("{}", output.join("report.json").display());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
