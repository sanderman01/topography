//! Diffusion world inputs: GLO-30 coarse base, authored class regions and game water.

use crate::acquisition::{
    AcquisitionError, Result, cover, digest, ensure, glo30, io_context, raster, read_small,
    write_new,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

const BLOCK_M: u32 = 32;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    class_id: u8,
    /// Offsets and sizes in metres from the world's north-west corner.
    x_m: u32,
    y_m: u32,
    width_m: u32,
    height_m: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldConfig {
    schema_version: u32,
    name: String,
    center_lon: f64,
    center_lat: f64,
    extent_m: u32,
    default_class: u8,
    #[serde(default)]
    regions: Vec<Region>,
}

fn unproject(srs: &str, x: f64, y: f64) -> Result<(f64, f64)> {
    let out = raster::run(raster::command("gdaltransform").args([
        "-s_srs",
        srs,
        "-t_srs",
        "EPSG:4326",
        "-output_xy",
        "-coord",
        &x.to_string(),
        &y.to_string(),
    ]))?;
    let text = String::from_utf8_lossy(&out);
    let v: Vec<f64> = text
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    ensure(
        v.len() >= 2,
        format!("unexpected gdaltransform output: {text}"),
    )?;
    Ok((v[0], v[1]))
}

/// Build `world.json`, `coarse.f32`, `classes.u8` and the 1 m `water.u8`, `snow-ice.u8`,
/// `vegetation.u8` and `land-cover.u8` in a new directory.
pub fn build_input(runtime: &Path, config_path: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "world input output already exists")?;
    crate::acquisition::geofabrik::use_runtime(runtime)?;
    let text = read_small(config_path)?;
    let config: WorldConfig = toml::from_str(&String::from_utf8_lossy(&text))
        .map_err(|e| AcquisitionError::Invalid(format!("world config: {e}")))?;
    ensure(
        config.schema_version == 1,
        "world config schema_version must be 1",
    )?;
    ensure(
        config.extent_m % 256 == 0 && (256..=16384).contains(&config.extent_m),
        "extent_m must be a multiple of 256 between 256 and 16384",
    )?;
    ensure(
        (-80.0..=84.0).contains(&config.center_lat) && (-180.0..180.0).contains(&config.center_lon),
        "center must lie inside UTM coverage",
    )?;
    ensure(config.default_class <= 9, "default_class must be 0..=9")?;
    for r in &config.regions {
        ensure(
            r.class_id <= 9
                && [r.x_m, r.y_m, r.width_m, r.height_m]
                    .iter()
                    .all(|v| v % BLOCK_M == 0)
                && r.x_m + r.width_m <= config.extent_m
                && r.y_m + r.height_m <= config.extent_m,
            "regions need class 0..=9, 32 m aligned bounds inside the extent",
        )?;
    }
    let zone = ((config.center_lon + 180.0) / 6.0).floor() as u32 + 1;
    let epsg = if config.center_lat >= 0.0 {
        32600
    } else {
        32700
    } + zone;
    let srs = format!("EPSG:{epsg}");
    let (cx, cy) = raster::project(&srs, config.center_lon, config.center_lat)?;
    let half = f64::from(config.extent_m) / 2.0;
    let snap = |v: f64| (v / f64::from(BLOCK_M)).round() * f64::from(BLOCK_M);
    let (x0, top) = (snap(cx - half), snap(cy + half));
    let e = f64::from(config.extent_m);
    let extent = [x0, top - e, x0 + e, top];
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (x, y) in [
        (extent[0], extent[1]),
        (extent[0], extent[3]),
        (extent[2], extent[1]),
        (extent[2], extent[3]),
    ] {
        let (lon, lat) = unproject(&srs, x, y)?;
        bbox = [
            bbox[0].min(lon),
            bbox[1].min(lat),
            bbox[2].max(lon),
            bbox[3].max(lat),
        ];
    }

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create world input parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage world input", tempfile::tempdir_in(parent))?;
    let scratch = io_context("world input scratch", tempfile::tempdir())?;

    let (coarse, base) = glo30::coarse(&srs, extent, f64::from(BLOCK_M), bbox, scratch.path())?;
    let blocks = (config.extent_m / BLOCK_M) as usize;
    let mut classes = vec![config.default_class; blocks * blocks];
    for r in &config.regions {
        let (bx, by) = ((r.x_m / BLOCK_M) as usize, (r.y_m / BLOCK_M) as usize);
        for row in by..by + (r.height_m / BLOCK_M) as usize {
            classes[row * blocks + bx..row * blocks + bx + (r.width_m / BLOCK_M) as usize]
                .fill(r.class_id);
        }
    }
    let wkt = String::from_utf8_lossy(&raster::run(raster::command("gdalsrsinfo").args([
        "-o",
        "wkt2_2019",
        "--single-line",
        &srs,
    ]))?)
    .trim()
    .to_owned();
    let cover = cover::world_cover(
        config.extent_m,
        config.extent_m,
        [x0, 1.0, 0.0, top, 0.0, -1.0],
        &wkt,
        bbox,
    )?;
    // Water keeps its 255 = water encoding; snow/ice and vegetation are 2 present / 1 absent.
    let water: Vec<u8> = cover
        .water
        .iter()
        .map(|&w| if w == 2 { 255 } else { 0 })
        .collect();
    let layers = [
        ("coarse.f32", &coarse),
        ("classes.u8", &classes),
        ("water.u8", &water),
        ("snow-ice.u8", &cover.snow),
        ("vegetation.u8", &cover.vegetation),
        ("land-cover.u8", &cover.land_cover),
    ];
    for (name, bytes) in layers {
        write_new(&stage.path().join(name), bytes)?;
    }
    let digests: serde_json::Map<String, serde_json::Value> = layers
        .iter()
        .map(|(name, bytes)| ((*name).to_owned(), json!(digest(bytes))))
        .collect();
    let manifest = json!({
        "schema_version": 2,
        "kind": "diffusion_world_input",
        "name": config.name,
        "size_blocks": [blocks, blocks],
        "block_m": BLOCK_M,
        "crs": srs,
        "origin": [x0, top],
        "extent_m": config.extent_m,
        "wgs84_bbox": bbox,
        "config_sha256": digest(&text),
        "config_text": String::from_utf8_lossy(&text),
        "base": base,
        "cover": cover.details,
        "layers": digests,
    });
    write_new(
        &stage.path().join("world.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    io_context("publish world input", fs::rename(stage.path(), output))?;
    Ok(output.join("world.json"))
}
