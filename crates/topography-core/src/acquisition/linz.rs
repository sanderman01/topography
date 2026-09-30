//! LINZ 1 m DEM discovery from the public nz-elevation STAC catalogue.
//!
//! Tiles follow the NZ Topo50 sheet grid (24 x 36 km sheets, lettered without I and
//! O) split into 5 x 5 tiles of 4,800 x 7,200 m, named like `CB08_10000_0104`. The
//! newest collection that has a tile wins. Each tile gets a complete v1 source
//! declaration and a metadata file holding its STAC collection and item.

use super::{AcquisitionError, Result, digest, ensure, http, io_context, raster, write_new};
use crate::catalog::source::{
    License, SourceRecord, Spatial, SurfaceType, TrainingPermission, validate_source,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const BASE: &str = "https://nz-elevation.s3.ap-southeast-2.amazonaws.com";
const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const TILE_W: f64 = 4_800.0;
const TILE_H: f64 = 7_200.0;
// Sheet 00 starts at this easting; sheet row AA ends at this northing.
const X0: f64 = 988_000.0;
const Y0: f64 = 6_810_000.0;
const CATALOG_MAX_AGE: Duration = Duration::from_secs(7 * 86_400);

/// Tile code containing an NZTM2000 point.
fn tile_code(e: f64, n: f64) -> Option<(String, [f64; 2])> {
    let sheet_x = ((e - X0) / 24_000.0).floor();
    let sheet_y = ((Y0 - n) / 36_000.0).floor();
    if !(0.0..100.0).contains(&sheet_x) || !(0.0..576.0).contains(&sheet_y) {
        return None;
    }
    let (left, top) = (X0 + sheet_x * 24_000.0, Y0 - sheet_y * 36_000.0);
    let col = ((e - left) / TILE_W).floor();
    let row = ((top - n) / TILE_H).floor();
    let k = sheet_y as usize;
    let code = format!(
        "{}{}{:02}_10000_{:02}{:02}",
        LETTERS[k / 24] as char,
        LETTERS[k % 24] as char,
        sheet_x as u32,
        row as u32 + 1,
        col as u32 + 1
    );
    Some((code, [left + col * TILE_W, top - row * TILE_H]))
}

fn fresh(path: &Path, max_age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < max_age)
}

/// Fetch through a local cache (collections change rarely; this spares the server).
fn cached(cache: &Path, url: &str, max_age: Duration) -> Result<Vec<u8>> {
    let path = cache.join(format!("{}.json", &digest(url.as_bytes())[..24]));
    if fresh(&path, max_age) {
        return io_context("read STAC cache", fs::read(&path));
    }
    let bytes = http::query(url)?;
    io_context("create STAC cache", fs::create_dir_all(cache))?;
    io_context("write STAC cache", fs::write(&path, &bytes))?;
    Ok(bytes)
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

#[derive(Debug, serde::Serialize)]
pub struct LinzDiscovery {
    pub schema_version: u32,
    pub provider: &'static str,
    pub query_bounds_wgs84: [f64; 4],
    pub tiles_in_bounds: usize,
    pub returned: usize,
    pub truncated: bool,
    pub source_files: Vec<PathBuf>,
}

/// CC BY 4.0 1 m DEM collections overlapping `bounds`, newest capture first, as
/// (end date, collection URL, collection).
fn collections(bounds: [f64; 4], cache: &Path) -> Result<Vec<(String, String, Value)>> {
    let catalog: Value = serde_json::from_slice(&cached(
        cache,
        &format!("{BASE}/catalog.json"),
        CATALOG_MAX_AGE,
    )?)?;
    let mut collections = Vec::new();
    for link in catalog["links"].as_array().into_iter().flatten() {
        let Some(href) = link["href"]
            .as_str()
            .filter(|h| h.contains("/dem_1m/2193/"))
        else {
            continue;
        };
        let url = format!("{BASE}/{}", href.trim_start_matches("./"));
        let collection: Value = serde_json::from_slice(&cached(cache, &url, CATALOG_MAX_AGE)?)?;
        let bbox: Option<[f64; 4]> =
            serde_json::from_value(collection["extent"]["spatial"]["bbox"][0].clone()).ok();
        if bbox.is_some_and(|b| overlaps(b, bounds)) && collection["license"] == "CC-BY-4.0" {
            let end = collection["extent"]["temporal"]["interval"][0][1]
                .as_str()
                .unwrap_or("")
                .to_owned();
            collections.push((end, url, collection));
        }
    }
    // Newest capture first.
    collections.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(collections)
}

/// Tile codes (with their NZTM origins) intersecting `bounds`.
fn codes(bounds: [f64; 4]) -> Result<Vec<(String, [f64; 2])>> {
    let corners = [
        raster::project("EPSG:2193", bounds[0], bounds[1])?,
        raster::project("EPSG:2193", bounds[0], bounds[3])?,
        raster::project("EPSG:2193", bounds[2], bounds[1])?,
        raster::project("EPSG:2193", bounds[2], bounds[3])?,
    ];
    let (e0, e1) = corners
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
    let (n0, n1) = corners
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
    let mut codes = Vec::new();
    let mut n = n1;
    while n > n0 - TILE_H {
        let mut e = e0;
        while e < e1 + TILE_W {
            if let Some((code, origin)) = tile_code(e.min(e1), n.max(n0))
                && !codes.iter().any(|(c, _)| *c == code)
            {
                codes.push((code, origin));
            }
            e += TILE_W;
        }
        n -= TILE_H;
    }
    Ok(codes)
}

/// Every published 1 m DEM tile URL intersecting `bounds`, oldest capture first so later
/// ones win where they have data. Also returns the collection slugs used.
pub(crate) fn dem_urls(bounds: [f64; 4], cache: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let collections = collections(bounds, cache)?;
    let codes = codes(bounds)?;
    let (mut urls, mut slugs) = (Vec::new(), Vec::new());
    for (_, url, collection) in collections.iter().rev() {
        let base = url.trim_end_matches("collection.json");
        let mut used = false;
        for (code, _) in &codes {
            if http::query_optional(&format!("{base}{code}.json"))?.is_some() {
                urls.push(format!("{base}{code}.tiff"));
                used = true;
            }
        }
        if used {
            slugs.push(
                collection["linz:slug"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned(),
            );
        }
    }
    Ok((urls, slugs))
}

/// Write one source declaration per LINZ 1 m DEM tile intersecting `bounds`
/// (lon/lat), at most `limit`. `cache` keeps STAC catalogue responses.
pub fn discover_linz(
    bounds: [f64; 4],
    limit: u32,
    output: &Path,
    cache: &Path,
) -> Result<LinzDiscovery> {
    ensure(
        (1..=100).contains(&limit),
        "discovery max-items must be in 1..=100",
    )?;
    ensure(
        bounds[0] < bounds[2] && bounds[1] < bounds[3] && bounds.iter().all(|v| v.is_finite()),
        "bbox must be ordered west,south,east,north degrees",
    )?;
    ensure(
        !output.exists(),
        "discovery output already exists; choose a new directory",
    )?;
    let collections = collections(bounds, cache)?;
    let codes = codes(bounds)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create discovery parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage discovery", tempfile::tempdir_in(parent))?;
    let mut summary = LinzDiscovery {
        schema_version: 1,
        provider: "linz",
        query_bounds_wgs84: bounds,
        tiles_in_bounds: codes.len(),
        returned: 0,
        truncated: false,
        source_files: Vec::new(),
    };
    for (code, _) in &codes {
        if summary.returned == limit as usize {
            summary.truncated = true;
            break;
        }
        for (end, url, collection) in &collections {
            let base = url.trim_end_matches("collection.json");
            let Some(bytes) = http::query_optional(&format!("{base}{code}.json"))? else {
                continue;
            };
            let item: Value = serde_json::from_slice(&bytes)?;
            let slug = collection["linz:slug"].as_str().unwrap_or("unknown");
            let start = collection["extent"]["temporal"]["interval"][0][0]
                .as_str()
                .unwrap_or("");
            let bbox: Option<[f64; 4]> = serde_json::from_value(item["bbox"].clone()).ok();
            let id = format!("linz-{slug}-{}", code.to_ascii_lowercase());
            let metadata_name = format!("{id}.json");
            let metadata =
                serde_json::to_vec_pretty(&json!({"collection": collection, "item": item}))?;
            write_new(&stage.path().join(&metadata_name), &metadata)?;
            let producers: Vec<&str> = collection["providers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["name"].as_str())
                .collect();
            let record = SourceRecord {
                schema_version: 1,
                record_id: id.clone(),
                synthetic: false,
                provider: "linz".into(),
                collection_id: collection["id"].as_str().unwrap_or("").into(),
                asset_id: code.clone(),
                survey_id: Some(slug.into()),
                acquisition_dates: vec![format!("{start}/{end}")],
                source_urls: vec![format!("{base}{code}.tiff")],
                original_metadata_path: metadata_name.into(),
                asset_sha256: None,
                spatial: Spatial {
                    bounds_wgs84: bbox,
                    horizontal_crs: Some("EPSG:2193".into()),
                    vertical_datum: Some("NZVD2016 (EPSG:7839), LINZ DEM collection".into()),
                    horizontal_units: Some("metre".into()),
                    vertical_units: Some("metre".into()),
                    native_spacing_m: Some([1.0, 1.0]),
                    ground_points_per_m2: None,
                    surface_type: SurfaceType::BareEarthDem,
                },
                processing_history: vec![format!(
                    "LINZ published LiDAR 1 m DEM ({}); STAC collection and item retained.",
                    collection["title"].as_str().unwrap_or(slug)
                )],
                license: License {
                    training_permission: TrainingPermission::Unknown,
                    license_name: Some("CC-BY-4.0 (LINZ collection declaration)".into()),
                    license_url: Some("https://creativecommons.org/licenses/by/4.0/".into()),
                    evidence: None,
                    attribution: Some(producers.join("; ")),
                    edited_map_reuse: None,
                    redistribution: None,
                },
            };
            validate_source(&record)?;
            let toml = toml::to_string_pretty(&record)
                .map_err(|e| AcquisitionError::Invalid(format!("encode source record: {e}")))?;
            let name = format!("{id}.toml");
            write_new(&stage.path().join(&name), toml.as_bytes())?;
            summary.source_files.push(PathBuf::from(name));
            summary.returned += 1;
            break;
        }
    }
    write_new(
        &stage.path().join("discovery.json"),
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    io_context("publish discovery", fs::rename(stage.path(), output))?;
    Ok(summary)
}
