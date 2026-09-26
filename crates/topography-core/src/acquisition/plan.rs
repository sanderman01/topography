//! Acquisition planning: named areas with a terrain class become a batch queue of
//! discovered sources and crop windows, without hand-written declarations.

use super::{AcquisitionError, Result, ensure, http, io_context, raster, write_new};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
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
    /// Offset of the window centre from the area centre, metres.
    offset: (f64, f64),
    distance_m: f64,
    bearing: &'static str,
    /// Survey end date; newer surveys win overlapping windows.
    survey_end: String,
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
    if dx.hypot(dy) < 150.0 {
        return "centre";
    }
    let names = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let angle = dx.atan2(dy).to_degrees().rem_euclid(360.0);
    names[((angle + 22.5) / 45.0) as usize % 8]
}

fn overlap_share(a: [f64; 4], b: [f64; 4]) -> f64 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let smaller = ((a[2] - a[0]) * (a[3] - a[1])).min((b[2] - b[0]) * (b[3] - b[1]));
    if smaller > 0.0 { w * h / smaller } else { 0.0 }
}

/// Discover sources for every area and write `queue.json` with crop windows nearest
/// each area's centre. Ground already curated (accepted or rejected) is skipped.
/// Returns the queue path.
pub fn plan(plan_path: &Path, runtime: &Path, output: &Path) -> Result<PathBuf> {
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
    let curated = crate::catalog::curation::curated_footprints(runtime)
        .map_err(|e| AcquisitionError::Invalid(format!("curated footprints: {e}")))?;
    let mut sources: Vec<Value> = Vec::new();
    let mut summary = Vec::new();
    for area in &plan.areas {
        let dir = output.join("discovery").join(slug(&area.name));
        let discovered = match area.provider.as_str() {
            "usgs" => super::discover_usgs(area.bbox, 100, &dir, None).map(|s| s.source_files),
            "linz" => super::discover_linz(area.bbox, 100, &dir, Path::new("data/cache/linz-stac"))
                .map(|s| s.source_files),
            other => Err(AcquisitionError::Invalid(format!(
                "unknown provider {other}"
            ))),
        };
        let files = match discovered {
            Ok(files) => files,
            Err(error) => {
                eprintln!("{}: discovery failed, area skipped: {error}", area.name);
                summary.push(json!({"area": area.name, "error": error.to_string()}));
                continue;
            }
        };
        let (cx, cy) = (
            (area.bbox[0] + area.bbox[2]) / 2.0,
            (area.bbox[1] + area.bbox[3]) / 2.0,
        );
        let near: Vec<[f64; 4]> = curated
            .iter()
            .filter(|b| overlap_share(**b, area.bbox) > 0.0)
            .copied()
            .collect();
        // Curated footprints projected per source CRS.
        let mut taken: HashMap<String, Vec<[f64; 4]>> = HashMap::new();
        let mut windows = Vec::new();
        let first_source = sources.len();
        for file in &files {
            let path = dir.join(file);
            let record: Value =
                toml::from_str(&String::from_utf8_lossy(&super::read_small(&path)?))
                    .map_err(|e| AcquisitionError::Invalid(format!("source: {e}")))?;
            let survey_end = record["acquisition_dates"][0]
                .as_str()
                .and_then(|d| d.split('/').next_back())
                .unwrap_or("")
                .to_owned();
            let (Some(url), Some(crs)) = (
                record["source_urls"][0].as_str(),
                record["spatial"]["horizontal_crs"].as_str(),
            ) else {
                continue; // incomplete declaration: cannot place crops
            };
            http::pace(url);
            let header = raster::run(
                raster::command("gdalinfo")
                    .args(["-json", "-nomd", "-norat", "-noct"])
                    .arg(format!("/vsicurl/{url}")),
            );
            let info: Value = match header {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) => {
                    eprintln!("{}: skipped, header unreadable: {error}", path.display());
                    continue;
                }
            };
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
            if !taken.contains_key(crs) {
                let mut boxes = Vec::new();
                for b in &near {
                    let corners = [
                        raster::project(crs, b[0], b[1])?,
                        raster::project(crs, b[0], b[3])?,
                        raster::project(crs, b[2], b[1])?,
                        raster::project(crs, b[2], b[3])?,
                    ];
                    boxes.push(
                        corners
                            .iter()
                            .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, c| {
                                [a[0].min(c.0), a[1].min(c.1), a[2].max(c.0), a[3].max(c.1)]
                            }),
                    );
                }
                taken.insert(crs.to_owned(), boxes);
            }
            let taken = &taken[crs];
            let index = sources.len();
            let mut added = false;
            let mut r = r0;
            while r < r1 {
                let mut c = c0;
                while c < c1 {
                    // Only whole windows: shifting edge windows inward duplicated neighbours.
                    if c + size > w || r + size > h {
                        c += size;
                        continue;
                    }
                    let (x, y) = (c, r);
                    let (x0, y0) = (t[0] + f64::from(x) * t[1], t[3] + f64::from(y) * t[5]);
                    let (x1, y1) = (x0 + f64::from(size) * t[1], y0 + f64::from(size) * t[5]);
                    let footprint = [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)];
                    if taken.iter().any(|b| overlap_share(*b, footprint) > 0.25) {
                        c += size;
                        continue;
                    }
                    let mx = t[0] + (f64::from(x) + f64::from(size) / 2.0) * t[1];
                    let my = t[3] + (f64::from(y) + f64::from(size) / 2.0) * t[5];
                    let (dx, dy) = (mx - centre.0, my - centre.1);
                    windows.push(Window {
                        source: index,
                        cells: [x, y, size, size],
                        offset: (dx, dy),
                        distance_m: dx.hypot(dy),
                        bearing: bearing(dx, dy),
                        survey_end: survey_end.clone(),
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
        // Nearest first; at equal distance (to 100 m) the newer survey first.
        windows.sort_by(|a, b| {
            (a.distance_m / 100.0)
                .round()
                .total_cmp(&(b.distance_m / 100.0).round())
                .then(b.survey_end.cmp(&a.survey_end))
        });
        // Overlapping surveys and neighbouring tiles cover the same ground: keep
        // windows that overlap no chosen window by more than a quarter.
        let limit = f64::from(size) * 0.5;
        let mut chosen: Vec<&Window> = Vec::new();
        for window in &windows {
            if chosen.len() == area.max_crops {
                break;
            }
            if chosen.iter().all(|c| {
                (c.offset.0 - window.offset.0).abs() >= limit
                    || (c.offset.1 - window.offset.1).abs() >= limit
            }) {
                chosen.push(window);
            }
        }
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
