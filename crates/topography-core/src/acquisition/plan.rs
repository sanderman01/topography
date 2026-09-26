//! Acquisition planning: named areas with a terrain class become a batch queue of
//! discovered sources and crop windows, without hand-written declarations.

use super::{AcquisitionError, Result, ensure, http, io_context, raster, write_new};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    #[serde(default = "default_crop")]
    crop_cells: u32,
    areas: Vec<Area>,
}

fn default_crop() -> u32 {
    2048
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Area {
    name: String,
    provider: String,
    /// west, south, east, north in degrees
    bbox: [f64; 4],
    class: u16,
    max_crops: usize,
}

struct Window {
    source: usize,
    cells: [u32; 4],
    distance_m: f64,
    bearing: &'static str,
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn bearing(dx: f64, dy: f64) -> &'static str {
    if dx.hypot(dy) < 500.0 {
        return "centre";
    }
    let names = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let angle = dx.atan2(dy).to_degrees().rem_euclid(360.0);
    names[((angle + 22.5) / 45.0) as usize % 8]
}

/// Discover sources for every area and write `queue.json` with crop windows nearest
/// each area's centre. Returns the queue path.
pub fn plan(plan_path: &Path, output: &Path) -> Result<PathBuf> {
    let plan: Plan = toml::from_str(&String::from_utf8_lossy(&super::read_small(plan_path)?))
        .map_err(|e| AcquisitionError::Invalid(format!("invalid plan: {e}")))?;
    ensure(
        plan.schema_version == 1 && (256..=2048).contains(&plan.crop_cells),
        "plan needs schema_version 1 and crop_cells in 256..=2048",
    )?;
    ensure(!output.exists(), "plan output already exists")?;
    io_context(
        "create plan output",
        fs::create_dir_all(output.join("discovery")),
    )?;
    let size = plan.crop_cells;
    let mut sources: Vec<Value> = Vec::new();
    let mut summary = Vec::new();
    for area in &plan.areas {
        let dir = output.join("discovery").join(slug(&area.name));
        let files: Vec<PathBuf> = match area.provider.as_str() {
            "usgs" => super::discover_usgs(area.bbox, 100, &dir, None)?.source_files,
            "linz" => {
                super::discover_linz(area.bbox, 100, &dir, Path::new("data/cache/linz-stac"))?
                    .source_files
            }
            other => {
                return Err(AcquisitionError::Invalid(format!(
                    "unknown provider {other}"
                )));
            }
        };
        let (cx, cy) = (
            (area.bbox[0] + area.bbox[2]) / 2.0,
            (area.bbox[1] + area.bbox[3]) / 2.0,
        );
        let mut windows = Vec::new();
        let first_source = sources.len();
        for file in &files {
            let path = dir.join(file);
            let record: Value =
                toml::from_str(&String::from_utf8_lossy(&super::read_small(&path)?))
                    .map_err(|e| AcquisitionError::Invalid(format!("source: {e}")))?;
            let (Some(url), Some(crs)) = (
                record["source_urls"][0].as_str(),
                record["spatial"]["horizontal_crs"].as_str(),
            ) else {
                continue; // incomplete declaration: cannot place crops
            };
            http::pace(url);
            let info: Value = serde_json::from_slice(&raster::run(
                raster::command("gdalinfo")
                    .args(["-json", "-nomd", "-norat", "-noct"])
                    .arg(format!("/vsicurl/{url}")),
            )?)?;
            let t: Vec<f64> = serde_json::from_value(info["geoTransform"].clone())?;
            let [w, h]: [u32; 2] = serde_json::from_value(info["size"].clone())?;
            let corners = [
                raster::project(crs, area.bbox[0], area.bbox[1])?,
                raster::project(crs, area.bbox[0], area.bbox[3])?,
                raster::project(crs, area.bbox[2], area.bbox[1])?,
                raster::project(crs, area.bbox[2], area.bbox[3])?,
            ];
            let centre = raster::project(crs, cx, cy)?;
            let col = |x: f64| ((x - t[0]) / t[1]).floor();
            let row = |y: f64| ((y - t[3]) / t[5]).floor();
            let c0 = corners
                .iter()
                .map(|c| col(c.0))
                .fold(f64::MAX, f64::min)
                .max(0.0) as u32;
            let c1 = corners
                .iter()
                .map(|c| col(c.0))
                .fold(f64::MIN, f64::max)
                .min(f64::from(w)) as u32;
            let r0 = corners
                .iter()
                .map(|c| row(c.1))
                .fold(f64::MAX, f64::min)
                .max(0.0) as u32;
            let r1 = corners
                .iter()
                .map(|c| row(c.1))
                .fold(f64::MIN, f64::max)
                .min(f64::from(h)) as u32;
            if w < size || h < size || c1 <= c0 || r1 <= r0 {
                continue;
            }
            let index = sources.len();
            let mut added = false;
            let mut r = r0;
            while r < r1 {
                let mut c = c0;
                while c < c1 {
                    // Windows stay whole inside the source grid.
                    let (x, y) = (c.min(w - size), r.min(h - size));
                    let mx = t[0] + (f64::from(x) + f64::from(size) / 2.0) * t[1];
                    let my = t[3] + (f64::from(y) + f64::from(size) / 2.0) * t[5];
                    let (dx, dy) = (mx - centre.0, my - centre.1);
                    windows.push(Window {
                        source: index,
                        cells: [x, y, size, size],
                        distance_m: dx.hypot(dy),
                        bearing: bearing(dx, dy),
                    });
                    added = true;
                    c += size;
                }
                r += size;
            }
            if added {
                sources.push(json!({
                    "id": format!("{}-{}", slug(&area.name), record["record_id"].as_str().unwrap_or("source")),
                    "source": fs::canonicalize(&path).map_err(|e| AcquisitionError::Io { context: "resolve source".into(), source: e })?,
                    "max_bytes": 2u64 << 30,
                    "crops": [],
                }));
            }
        }
        windows.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
        windows.dedup_by(|a, b| a.source == b.source && a.cells == b.cells);
        let chosen: Vec<&Window> = windows.iter().take(area.max_crops).collect();
        for (n, window) in chosen.iter().enumerate() {
            let title = if window.bearing == "centre" {
                format!("{} · centre", area.name)
            } else {
                format!(
                    "{} · {} {:.1} km",
                    area.name,
                    window.bearing,
                    window.distance_m / 1000.0
                )
            };
            let crops = sources[window.source]["crops"]
                .as_array_mut()
                .expect("crops array");
            crops.push(json!({
                "id": format!("crop-{:02}", n + 1),
                "title": title,
                "window_cells": window.cells,
                "primary_class_id": area.class,
            }));
        }
        summary.push(json!({"area": area.name, "sources": files.len(), "candidate_windows": windows.len(), "crops": chosen.len()}));
        // Sources without chosen crops are not downloaded.
        let kept: Vec<Value> = sources
            .drain(first_source..)
            .filter(|s| s["crops"].as_array().is_some_and(|c| !c.is_empty()))
            .collect();
        sources.extend(kept);
    }
    ensure(!sources.is_empty(), "no crops planned")?;
    let total: u64 = sources.len() as u64 * (2u64 << 30);
    let queue = json!({"schema_version": 1, "max_total_bytes": total, "sources": sources});
    let path = output.join("queue.json");
    write_new(&path, &serde_json::to_vec_pretty(&queue)?)?;
    write_new(
        &output.join("plan-summary.json"),
        &serde_json::to_vec_pretty(&json!({"areas": summary}))?,
    )?;
    Ok(path)
}
