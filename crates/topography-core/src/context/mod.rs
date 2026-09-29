//! GLO-30 context regions: 32 m real terrain over tens of kilometres with game cover and
//! class labels, the training data for the coarse cascade levels (L1 128 m, L2 32 m).

pub mod gallery;

use crate::acquisition::{
    AcquisitionError, Result, cover, digest, ensure, glo30, io_context, raster, read_small,
    write_new,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) const SPACING_M: u32 = 32;
const DEFAULT_EXTENT_M: u32 = 49_152;
pub(crate) const LAYERS: [&str; 6] = [
    "height.f32",
    "classes.u8",
    "water.u8",
    "snow-ice.u8",
    "vegetation.u8",
    "land-cover.u8",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Area {
    class_id: u8,
    /// Outline as [lon, lat] vertices; later areas override earlier ones.
    polygon: Vec<[f64; 2]>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RegionPlan {
    id: String,
    /// Class of the whole region unless an area overrides it.
    class_id: u8,
    /// [lon, lat] of the region centre.
    center: [f64; 2],
    reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extent_m: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    areas: Vec<Area>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    name: String,
    regions: Vec<RegionPlan>,
}

fn load_plan(path: &Path) -> Result<(Vec<u8>, Plan)> {
    let text = read_small(path)?;
    let plan: Plan = toml::from_str(&String::from_utf8_lossy(&text))
        .map_err(|e| AcquisitionError::Invalid(format!("context plan: {e}")))?;
    ensure(
        plan.schema_version == 1,
        "context plan schema_version must be 1",
    )?;
    let mut ids = std::collections::BTreeSet::new();
    for r in &plan.regions {
        ensure(
            !r.id.is_empty()
                && r.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && ids.insert(r.id.as_str()),
            format!("region ids must be unique lowercase slugs: {}", r.id),
        )?;
        let extent = r.extent_m.unwrap_or(DEFAULT_EXTENT_M);
        ensure(
            extent % 1024 == 0 && (16_384..=98_304).contains(&extent),
            format!(
                "{}: extent_m must be a multiple of 1024 from 16384 to 98304",
                r.id
            ),
        )?;
        ensure(
            (-80.0..=84.0).contains(&r.center[1]) && (-180.0..180.0).contains(&r.center[0]),
            format!("{}: centre must lie inside UTM coverage", r.id),
        )?;
        ensure(
            (1..=12).contains(&r.class_id)
                && r.areas
                    .iter()
                    .all(|a| a.class_id <= 12 && a.polygon.len() >= 3),
            format!(
                "{}: classes must be 1..=12 (areas 0..=12) with 3+ vertices",
                r.id
            ),
        )?;
    }
    Ok((text, plan))
}

/// Even-odd point-in-polygon test.
fn inside(poly: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut hit = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let ((xi, yi), (xj, yj)) = (poly[i], poly[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            hit = !hit;
        }
        j = i;
    }
    hit
}

pub(crate) fn heights(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Hillshade (0..=255) for light from `azimuth` degrees clockwise from north at 45°.
pub(crate) fn hillshade(h: &[f32], w: usize, rows: usize, spacing: f32, azimuth: f32) -> Vec<u8> {
    let (az, alt) = (azimuth.to_radians(), 45f32.to_radians());
    let light = [az.sin() * alt.cos(), az.cos() * alt.cos(), alt.sin()];
    let at = |x: isize, y: isize| {
        h[y.clamp(0, rows as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize]
    };
    let mut out = Vec::with_capacity(w * rows);
    for y in 0..rows as isize {
        for x in 0..w as isize {
            let east = (at(x + 1, y) - at(x - 1, y)) / (2.0 * spacing);
            // Rows run southward.
            let north = (at(x, y - 1) - at(x, y + 1)) / (2.0 * spacing);
            let n = [-east, -north, 1.0];
            let len = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
            let d = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / len;
            out.push((d.max(0.0) * 255.0) as u8);
        }
    }
    out
}

/// Tinted hillshade: land from green through brown to white, water blue.
fn overview(h: &[f32], water: &[u8], w: usize, rows: usize, low: f32, high: f32) -> Vec<u8> {
    let shade = hillshade(h, w, rows, SPACING_M as f32, 315.0);
    let stops: [(f32, [f32; 3]); 4] = [
        (0.0, [90.0, 130.0, 80.0]),
        (0.4, [170.0, 160.0, 110.0]),
        (0.75, [150.0, 115.0, 90.0]),
        (1.0, [245.0, 245.0, 245.0]),
    ];
    let mut rgb = Vec::with_capacity(w * rows * 3);
    for i in 0..w * rows {
        let base = if water[i] == 255 {
            [60.0, 110.0, 170.0]
        } else {
            let t = ((h[i] - low) / (high - low).max(1.0)).clamp(0.0, 1.0);
            let k = stops.iter().position(|s| s.0 >= t).unwrap_or(3).max(1);
            let ((t0, a), (t1, b)) = (stops[k - 1], stops[k]);
            let f = (t - t0) / (t1 - t0);
            [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * f)
        };
        let light = 0.35 + 0.65 * f32::from(shade[i]) / 255.0;
        rgb.extend(base.map(|c| (c * light).min(255.0) as u8));
    }
    rgb
}

fn png_rgb(width: usize, height: usize, rgb: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| AcquisitionError::Invalid(format!("png: {e}")))?;
        writer
            .write_image_data(rgb)
            .map_err(|e| AcquisitionError::Invalid(format!("png: {e}")))?;
    }
    Ok(out)
}

fn percentile(sorted: &[f32], p: f64) -> f32 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Land height range, relief and flat share (slope under 3°) at 32 m.
fn stats(h: &[f32], water: &[u8], w: usize, rows: usize) -> Value {
    let mut land: Vec<f32> = h
        .iter()
        .zip(water)
        .filter(|(_, wet)| **wet != 255)
        .map(|(v, _)| *v)
        .collect();
    land.sort_by(f32::total_cmp);
    let flat_limit = 3f32.to_radians().tan() * 2.0 * SPACING_M as f32;
    let (mut flat, mut counted) = (0usize, 0usize);
    for y in 1..rows - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if water[i] == 255 {
                continue;
            }
            let (dx, dy) = (h[i + 1] - h[i - 1], h[i + w] - h[i - w]);
            counted += 1;
            flat += usize::from((dx * dx + dy * dy).sqrt() < flat_limit);
        }
    }
    let cells = (w * rows) as f64;
    if land.is_empty() {
        return json!({"land_share": 0.0});
    }
    json!({
        "land_share": land.len() as f64 / cells,
        "min_m": land[0], "max_m": land[land.len() - 1],
        "p5_m": percentile(&land, 0.05), "p95_m": percentile(&land, 0.95),
        "flat_share": flat as f64 / counted.max(1) as f64,
    })
}

/// Existing region output: Ok(true) when it matches the plan entry, Ok(false) when stale.
fn verify_existing(dir: &Path, config_sha256: &str) -> Result<bool> {
    let manifest: Value = serde_json::from_slice(&read_small(&dir.join("region.json"))?)?;
    if manifest["region_config_sha256"] != config_sha256 {
        return Ok(false);
    }
    for name in LAYERS.iter().chain(&["overview.png"]) {
        let bytes = io_context(format!("read {name}"), fs::read(dir.join(name)))?;
        ensure(
            manifest["files"][name].as_str() == Some(digest(&bytes).as_str()),
            format!("{} does not match region.json", dir.join(name).display()),
        )?;
    }
    Ok(true)
}

fn build_region(plan_name: &str, plan_sha256: &str, r: &RegionPlan, output: &Path) -> Result<()> {
    let extent_m = r.extent_m.unwrap_or(DEFAULT_EXTENT_M);
    let [lon, lat] = r.center;
    let zone = ((lon + 180.0) / 6.0).floor() as u32 + 1;
    let srs = format!("EPSG:{}", if lat >= 0.0 { 32600 } else { 32700 } + zone);
    let (cx, cy) = raster::project(&srs, lon, lat)?;
    let s = f64::from(SPACING_M);
    let e = f64::from(extent_m);
    let snap = |v: f64| (v / s).round() * s;
    let (x0, top) = (snap(cx - e / 2.0), snap(cy + e / 2.0));
    let extent = [x0, top - e, x0 + e, top];
    let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (x, y) in [
        (extent[0], extent[1]),
        (extent[0], extent[3]),
        (extent[2], extent[1]),
        (extent[2], extent[3]),
    ] {
        let (lon, lat) = raster::unproject(&srs, x, y)?;
        bbox = [
            bbox[0].min(lon),
            bbox[1].min(lat),
            bbox[2].max(lon),
            bbox[3].max(lat),
        ];
    }
    let n = (extent_m / SPACING_M) as usize;
    let scratch = io_context("context scratch", tempfile::tempdir())?;
    let (height_bytes, base) = glo30::coarse(&srs, extent, s, bbox, scratch.path())?;
    let wkt = String::from_utf8_lossy(&raster::run(raster::command("gdalsrsinfo").args([
        "-o",
        "wkt2_2019",
        "--single-line",
        &srs,
    ]))?)
    .trim()
    .to_owned();
    let cover = cover::world_cover(n as u32, n as u32, [x0, s, 0.0, top, 0.0, -s], &wkt, bbox)?;

    let mut classes = vec![r.class_id; n * n];
    for area in &r.areas {
        let poly = area
            .polygon
            .iter()
            .map(|[lon, lat]| raster::project(&srs, *lon, *lat))
            .collect::<Result<Vec<_>>>()?;
        for (i, c) in classes.iter_mut().enumerate() {
            let (x, y) = (
                x0 + ((i % n) as f64 + 0.5) * s,
                top - ((i / n) as f64 + 0.5) * s,
            );
            if inside(&poly, x, y) {
                *c = area.class_id;
            }
        }
    }
    // Same encodings as world inputs: water 255, snow/ice and vegetation 2 present / 1 absent.
    let water: Vec<u8> = cover
        .water
        .iter()
        .map(|&w| if w == 2 { 255 } else { 0 })
        .collect();
    let h = heights(&height_bytes);
    let stats = stats(&h, &water, n, n);
    let (low, high) = (
        stats["p5_m"].as_f64().unwrap_or(0.0) as f32,
        stats["max_m"].as_f64().unwrap_or(1.0) as f32,
    );
    let overview_png = png_rgb(n, n, &overview(&h, &water, n, n, low, high))?;

    io_context("create context output", fs::create_dir_all(output))?;
    let stage = io_context("stage context region", tempfile::tempdir_in(output))?;
    let layers: [(&str, &[u8]); 7] = [
        (LAYERS[0], &height_bytes),
        (LAYERS[1], &classes),
        (LAYERS[2], &water),
        (LAYERS[3], &cover.snow),
        (LAYERS[4], &cover.vegetation),
        (LAYERS[5], &cover.land_cover),
        ("overview.png", &overview_png),
    ];
    let mut files = serde_json::Map::new();
    for (name, bytes) in layers {
        write_new(&stage.path().join(name), bytes)?;
        files.insert(name.into(), json!(digest(bytes)));
    }
    let config = serde_json::to_vec(r)?;
    let manifest = json!({
        "schema_version": 1,
        "kind": "context_region",
        "id": r.id,
        "class_id": r.class_id,
        "reference": r.reference,
        "plan": {"name": plan_name, "sha256": plan_sha256},
        "region_config": serde_json::from_slice::<Value>(&config)?,
        "region_config_sha256": digest(&config),
        "spacing_m": SPACING_M,
        "size": [n, n],
        "crs": srs,
        "origin": [x0, top],
        "wgs84_bbox": bbox,
        "encoding": "row-major from the north-west corner; height Float32 LE metres (EGM2008), \
            0 m where GLO-30 has no tile; classes u8; water 255/0; snow-ice and vegetation 2 present / 1 absent; \
            land-cover WorldCover classes",
        "base": base,
        "cover": cover.details,
        "stats": stats,
        "files": files,
        "admission": "unreviewed until a Yes in context serve",
    });
    write_new(
        &stage.path().join("region.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    io_context(
        "publish context region",
        fs::rename(stage.path(), output.join(&r.id)),
    )
}

/// Build every planned region missing from `output`, reusing verified ones. Returns a JSON
/// summary; any failed or stale region makes the command fail after the others finish.
pub fn build(runtime: &Path, plan_path: &Path, output: &Path) -> Result<String> {
    crate::acquisition::geofabrik::use_runtime(runtime)?;
    let (text, plan) = load_plan(plan_path)?;
    let plan_sha256 = digest(&text);
    let (mut built, mut reused, mut stale, mut failed) = (vec![], vec![], vec![], vec![]);
    for (i, r) in plan.regions.iter().enumerate() {
        let dir = output.join(&r.id);
        let config_sha256 = digest(&serde_json::to_vec(r)?);
        if dir.exists() {
            match verify_existing(&dir, &config_sha256) {
                Ok(true) => reused.push(r.id.clone()),
                Ok(false) => stale.push(r.id.clone()),
                Err(e) => failed.push(json!({"id": r.id, "error": e.to_string()})),
            }
            continue;
        }
        eprintln!("[{}/{}] {}", i + 1, plan.regions.len(), r.id);
        match build_region(&plan.name, &plan_sha256, r, output) {
            Ok(()) => built.push(r.id.clone()),
            Err(e) => {
                eprintln!("  failed: {e}");
                failed.push(json!({"id": r.id, "error": e.to_string()}));
            }
        }
    }
    let summary = serde_json::to_string_pretty(&json!({
        "plan": plan.name, "output": output.display().to_string(),
        "built": built, "reused": reused, "failed": failed,
        "stale": stale,
        "stale_note": "plan entry changed since the region was built; give it a new id or delete the old region explicitly",
    }))?;
    if failed.is_empty() && stale.is_empty() {
        Ok(summary)
    } else {
        Err(AcquisitionError::Invalid(summary))
    }
}

/// Region directories under `root` with a region manifest, sorted by id.
pub(crate) fn regions(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = io_context("list context regions", fs::read_dir(root))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("region.json").is_file())
        .collect();
    dirs.sort();
    Ok(dirs)
}
