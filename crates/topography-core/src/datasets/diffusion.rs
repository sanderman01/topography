//! Diffusion training release: a snapshot of every accepted Ready gallery tile.

use super::*;
use crate::{
    acquisition::io_context,
    catalog::curation,
    preparation::automatic,
    review::{artifacts, band, gallery},
};
use std::fs;

const REPORT_WINDOW: usize = 256;

const LAYERS: [(&str, bool); 6] = [
    ("elevation", true),
    ("validity", false),
    ("water", false),
    ("snow-ice", false),
    ("vegetation", false),
    ("land-cover", false),
];

fn footprint(grid: &Grid) -> [f64; 4] {
    let right = grid.origin[0] + f64::from(grid.size[0]) * grid.spacing_m[0];
    let bottom = grid.origin[1] - f64::from(grid.size[1]) * grid.spacing_m[1];
    [grid.origin[0], bottom, right, grid.origin[1]]
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Publish raw layers of the current prepared product of each accepted Ready tile.
pub fn build_diffusion(runtime: &Path, gallery_path: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "release output already exists")?;
    let (gallery, gallery_sha256, snapshots) = gallery::load(gallery_path)?;
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for (tile, snapshot) in gallery.tiles.iter().zip(&snapshots) {
        if curation::selection(runtime, &tile.review_sha256, snapshot)?["decision"] != "accept" {
            continue;
        }
        // Same readiness rule as the gallery worker: verified product with current game cover.
        let Some(path) = automatic::existing(runtime, &tile.review_sha256)? else {
            continue;
        };
        if !automatic::current_cover(&path) {
            continue;
        }
        let bytes = read_small(&path)?;
        let id = digest(&bytes);
        if !seen.insert(id.clone()) {
            continue;
        }
        let manifest: Value = serde_json::from_slice(&bytes)?;
        let grid: Grid = serde_json::from_value(manifest["grid"].clone())?;
        candidates.push((tile, id, path, manifest, grid));
    }
    // Larger products win footprint overlaps so no terrain is counted twice.
    candidates.sort_by(|a, b| {
        let area = |g: &Grid| {
            f64::from(g.size[0]) * g.spacing_m[0] * f64::from(g.size[1]) * g.spacing_m[1]
        };
        area(&b.4).total_cmp(&area(&a.4)).then(a.1.cmp(&b.1))
    });
    let mut kept: Vec<(Value, [f64; 4])> = Vec::new();
    let mut skipped = Vec::new();
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create release parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage release", tempfile::tempdir_in(parent))?;
    let scratch = io_context("release scratch", tempfile::tempdir())?;
    let mut tiles = Vec::new();
    let mut report = Vec::new();
    for (tile, id, path, manifest, grid) in candidates {
        let extent = footprint(&grid);
        if kept
            .iter()
            .any(|(crs, other)| *crs == manifest["coordinate_system"] && overlaps(extent, *other))
        {
            skipped.push(json!({"prepared_sha256": id, "reason": "overlaps a larger product"}));
            continue;
        }
        ensure(
            (1..=12).contains(&tile.primary_class_id),
            "tile class must be a legend class 1..=12",
        )?;
        let (w, rows) = (grid.size[0] as usize, grid.size[1] as usize);
        let cells = w * rows;
        let mut raw = BTreeMap::new();
        for (layer, float) in LAYERS {
            let bytes = band(
                &path.with_file_name(format!("{layer}.tif")),
                scratch.path(),
                layer,
                cells,
                float,
            )?;
            raw.insert(layer, bytes);
        }
        // Artifact checks on valid ground; stripes are repaired, the rest is masked.
        let mut height: Vec<f32> = raw["elevation"]
            .chunks_exact(4)
            .zip(&raw["validity"])
            .map(|(b, v)| {
                if *v == 255 {
                    f32::from_le_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    f32::NAN
                }
            })
            .collect();
        let detection = artifacts::detect_metre(&height, w, rows, grid.spacing_m[0]);
        let original = height.clone();
        artifacts::destripe(&mut height, w, rows, &detection.stripes);
        artifacts::defacet(&mut height, &detection.mask, w, rows, grid.spacing_m[0]);
        let summary = artifacts::summary(&original, &detection, grid.spacing_m);
        let elevation: Vec<u8> = raw["elevation"]
            .chunks_exact(4)
            .zip(&height)
            .flat_map(|(b, h)| {
                if h.is_finite() {
                    h.to_le_bytes()
                } else {
                    [b[0], b[1], b[2], b[3]]
                }
            })
            .collect();
        raw.insert("elevation", elevation);
        raw.insert("artifacts", detection.mask.clone());
        report.push(report_entry(
            stage.path(),
            &id,
            tile.title.as_str(),
            &summary,
            (&original, &height, &detection.mask),
            w,
            rows,
        )?);
        let mut layers = serde_json::Map::new();
        for (layer, float) in LAYERS.into_iter().chain([("artifacts", false)]) {
            let bytes = &raw[layer];
            let name = format!("{id}/{layer}.{}", if float { "f32" } else { "u8" });
            write_new(&stage.path().join(&name), bytes)?;
            layers.insert(
                layer.into(),
                json!({"path": name, "sha256": digest(bytes), "dtype": if float {"float32_le"} else {"uint8"}}),
            );
        }
        tiles.push(json!({
            "prepared_sha256": id,
            "review_sha256": tile.review_sha256,
            "title": tile.title,
            "primary_class_id": tile.primary_class_id,
            "survey": manifest["survey"],
            "coordinate_system": manifest["coordinate_system"],
            "grid": grid,
            "masked_fill_m": manifest["masked_fill_m"],
            "artifacts": summary,
            "repairs": "elevation.f32 has listed stripe periods removed (fir_notch_v1) and facet cells smoothed (box3_blend_v1)",
            "layers": layers,
        }));
        kept.push((manifest["coordinate_system"].clone(), extent));
    }
    ensure(!tiles.is_empty(), "no accepted Ready tiles in this gallery")?;
    write_new(
        &stage.path().join("report.html"),
        report_page(&report).as_bytes(),
    )?;
    let release = json!({
        "schema_version": 2,
        "kind": "diffusion_training_release",
        "gallery_sha256": gallery_sha256,
        "topography_version": env!("CARGO_PKG_VERSION"),
        "mask_codes": {
            "validity": "255 valid ground, 0 masked",
            "water": "255 water, 0 not water",
            "snow-ice": "2 present, 1 absent, 0 unknown",
            "vegetation": "2 present, 1 absent, 0 unknown",
            "land-cover": "ESA WorldCover v200 class (10 tree, 20 shrub, 30 grass, 40 crop, 50 built-up, 60 bare/sparse, 70 snow/ice, 80 water, 90 wetland, 95 mangrove, 100 moss/lichen), 0 outside coverage",
            "artifacts": "bit flags: 1 interpolated facet (smoothed), 2 periodic stripe (filtered), 4 straight seam (excluded from training)",
        },
        "artifact_checks": artifacts::CHECKS,
        "report": "report.html",
        "tiles": tiles,
        "skipped": skipped,
    });
    write_new(
        &stage.path().join("manifest.json"),
        &serde_json::to_vec_pretty(&release)?,
    )?;
    io_context("publish release", fs::rename(stage.path(), output))?;
    Ok(output.join("manifest.json"))
}

/// Grey hillshade (light from the north-west) of one window, artifact cells tinted.
fn hillshade(height: &[f32], mask: &[u8], w: usize, x0: usize, y0: usize, size: usize) -> Vec<u8> {
    let at = |x: usize, y: usize| {
        let v = height[y * w + x];
        if v.is_finite() {
            Some(f64::from(v))
        } else {
            None
        }
    };
    let mut rgb = Vec::with_capacity(size * size * 3);
    for y in y0..y0 + size {
        for x in x0..x0 + size {
            let (xl, xr, yu, yd) = (
                x.saturating_sub(1),
                (x + 1).min(w - 1),
                y.saturating_sub(1),
                y + 1,
            );
            let shade = match (
                at(xl, y),
                at(xr, y),
                at(x, yu),
                at(x, yd.min(height.len() / w - 1)),
            ) {
                (Some(l), Some(r), Some(u), Some(d)) => {
                    let (gx, gy) = ((r - l) / 2.0, (d - u) / 2.0);
                    let n = (gx * gx + gy * gy + 1.0).sqrt();
                    // Normal (-gx, -gy, 1) against light from the north-west at 45 degrees.
                    ((0.5 * gx + 0.5 * gy + std::f64::consts::FRAC_1_SQRT_2) / n).clamp(0.0, 1.0)
                        * 235.0
                        + 20.0
                }
                _ => 12.0,
            };
            let m = mask[y * w + x];
            // Stripes and facets are repaired, so only excluded seams are tinted.
            let tint = if m & artifacts::SEAM != 0 {
                [1.0, 0.45, 0.45]
            } else {
                [1.0, 1.0, 1.0]
            };
            rgb.extend(tint.map(|t| (shade * t) as u8));
        }
    }
    rgb
}

fn write_png(path: &Path, size: usize, rgb: &[u8]) -> Result<()> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, size as u32, size as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| AcquisitionError::Invalid(format!("png: {e}")))?;
        writer
            .write_image_data(rgb)
            .map_err(|e| AcquisitionError::Invalid(format!("png: {e}")))?;
    }
    write_new(path, &out)
}

/// Close-up of the window with the most flagged cells: source and training versions.
fn report_entry(
    stage: &Path,
    id: &str,
    title: &str,
    summary: &Value,
    (original, repaired, mask): (&[f32], &[f32], &[u8]),
    w: usize,
    rows: usize,
) -> Result<Value> {
    let size = REPORT_WINDOW.min(w).min(rows);
    let (mut best, mut at) = (0usize, (0usize, 0usize));
    for y0 in (0..=rows - size).step_by(64) {
        for x0 in (0..=w - size).step_by(64) {
            let n = (y0..y0 + size)
                .map(|y| {
                    mask[y * w + x0..y * w + x0 + size]
                        .iter()
                        .filter(|m| **m != 0)
                        .count()
                })
                .sum::<usize>();
            if n > best {
                (best, at) = (n, (x0, y0));
            }
        }
    }
    let mut images = Vec::new();
    if best > 0 {
        io_context("create report", fs::create_dir_all(stage.join("report")))?;
        for (name, data) in [("source", original), ("training", repaired)] {
            let file = format!("report/{id}-{name}.png");
            write_png(
                &stage.join(&file),
                size,
                &hillshade(data, mask, w, at.0, at.1, size),
            )?;
            images.push(file);
        }
    }
    Ok(json!({"title": title, "summary": summary, "window": [at.0, at.1, size], "images": images}))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn report_page(entries: &[Value]) -> String {
    let pct = |v: &Value| format!("{:.1}%", v.as_f64().unwrap_or(0.0) * 100.0);
    let mut rows = String::new();
    let mut shots = String::new();
    for e in entries {
        let s = &e["summary"];
        let stripes: Vec<String> = s["stripes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                format!(
                    "{} m along {}",
                    t["period_m"],
                    t["axis"].as_str().unwrap_or("")
                )
            })
            .collect();
        let title = escape(e["title"].as_str().unwrap_or(""));
        rows.push_str(&format!(
            "<tr><td>{title}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            pct(&s["facet_fraction"]),
            pct(&s["seam_fraction"]),
            if stripes.is_empty() {
                "none".into()
            } else {
                format!("{} (repaired)", stripes.join(", "))
            },
            pct(&s["excluded_fraction"]),
        ));
        if let [source, training] = e["images"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            shots.push_str(&format!(
                "<figure><figcaption>{title} · worst {size} m window at cell {x}, {y}</figcaption><img src=\"{}\" alt=\"source\"><img src=\"{}\" alt=\"training\"><p>Left: source. Right: training copy (stripes filtered, facets smoothed). Red: seams excluded from training.</p></figure>",
                source.as_str().unwrap_or(""),
                training.as_str().unwrap_or(""),
                size = e["window"][2],
                x = e["window"][0],
                y = e["window"][1],
            ));
        }
    }
    format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Release artifacts</title><style>:root{{color-scheme:dark;background:#111917;color:#e6eee8;font:15px system-ui,sans-serif}}body{{max-width:1100px;margin:auto;padding:16px}}table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #34453d;padding:6px;text-align:left}}figure{{margin:24px 0}}img{{width:min(49%,512px);image-rendering:pixelated;margin-right:1%}}p{{color:#a0b2a7}}</style><h1>Release artifact report</h1><p>Automatic checks ({}) on each tile's native grid. The training copy has stripes filtered out and facets smoothed; seam cells are left out of training.</p><table><tr><th>Tile</th><th>Facets (smoothed)</th><th>Seams</th><th>Stripes</th><th>Excluded</th></tr>{rows}</table>{shots}</html>",
        artifacts::CHECKS
    )
}
