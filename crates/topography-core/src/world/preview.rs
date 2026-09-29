//! Coarse preview of a world input: its 32 m surface interpolated to a few metres plus sampled
//! cover masks, in the world output layout so `export heightmap` and `export view` can show it
//! before any GPU generation.

use crate::acquisition::{Result, digest, ensure, io_context, read_small, write_new};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Write `elevation.f32` and masks at `spacing_m` (a power of two below the 32 m block).
pub fn build_preview(world_input: &Path, output: &Path, spacing_m: usize) -> Result<PathBuf> {
    ensure(!output.exists(), "preview output already exists")?;
    let spec: Value = serde_json::from_slice(&read_small(&world_input.join("world.json"))?)?;
    ensure(
        spec["kind"] == "diffusion_world_input" && spec["block_m"] == 32,
        "not a 32 m world input",
    )?;
    ensure(
        spacing_m.is_power_of_two() && (1..=16).contains(&spacing_m),
        "spacing must be a power of two from 1 to 16 m",
    )?;
    let [cols, rows]: [usize; 2] = serde_json::from_value(spec["size_blocks"].clone())?;
    let (w1, h1) = (cols * 32, rows * 32);
    let (w, h) = (w1 / spacing_m, h1 / spacing_m);
    let layers = &spec["layers"];
    let read = |name: &str, size: usize| -> Result<Vec<u8>> {
        let bytes = io_context(format!("read {name}"), fs::read(world_input.join(name)))?;
        ensure(
            bytes.len() == size && layers[name].as_str() == Some(digest(&bytes).as_str()),
            format!("{name} does not match world.json"),
        )?;
        Ok(bytes)
    };
    let coarse: Vec<f32> = read("coarse.f32", cols * rows * 4)?
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();

    // Bilinear between 32 m block centres, clamped at the edges.
    let mut elevation = Vec::with_capacity(w * h * 4);
    let scale = spacing_m as f32 / 32.0;
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * scale - 0.5).clamp(0.0, (rows - 1) as f32);
        let (y0, ty) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(rows - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * scale - 0.5).clamp(0.0, (cols - 1) as f32);
            let (x0, tx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(cols - 1);
            let v = |xx: usize, yy: usize| coarse[yy * cols + xx];
            let top = v(x0, y0) + (v(x1, y0) - v(x0, y0)) * tx;
            let bottom = v(x0, y1) + (v(x1, y1) - v(x0, y1)) * tx;
            elevation.extend_from_slice(&(top + (bottom - top) * ty).to_le_bytes());
        }
    }

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create preview parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage preview", tempfile::tempdir_in(parent))?;
    let mut digests = serde_json::Map::new();
    write_new(&stage.path().join("elevation.f32"), &elevation)?;
    digests.insert("elevation.f32".into(), json!(digest(&elevation)));
    let half = spacing_m / 2;
    for name in ["water.u8", "snow-ice.u8", "vegetation.u8", "land-cover.u8"] {
        if layers.get(name).is_none() {
            continue;
        }
        // Nearest 1 m cell to each preview cell centre.
        let full = read(name, w1 * h1)?;
        let sampled: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (y, x)))
            .map(|(y, x)| full[(y * spacing_m + half) * w1 + x * spacing_m + half])
            .collect();
        write_new(&stage.path().join(name), &sampled)?;
        digests.insert(name.into(), json!(digest(&sampled)));
    }
    let manifest = json!({
        "schema_version": 1,
        "kind": "world_preview",
        "world_input": world_input.canonicalize().map(|p| p.display().to_string()).unwrap_or_default(),
        "world_input_sha256": digest(&read_small(&world_input.join("world.json"))?),
        "spacing_m": spacing_m as f64,
        "size": [w, h],
        "method": "32 m coarse surface, bilinear between block centres; masks sampled nearest from 1 m",
        "water_fill": "coarse surface",
        "layers": digests,
    });
    write_new(
        &stage.path().join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    io_context("publish preview", fs::rename(stage.path(), output))?;
    Ok(output.join("manifest.json"))
}
