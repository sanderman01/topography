//! Public 1 m lidar DEMs (USGS 3DEP, LINZ) averaged straight to a coarser metric grid.
//! Only the reduced grid is kept; the source list makes it reproducible.

use super::{AcquisitionError, Result, ensure, http, io_context, linz, raster::run};
use crate::review;
use serde_json::{Value, json};
use std::path::Path;

const PARALLEL_DOWNLOADS: usize = 4;

/// TNM 1 m DEM tile URLs in `bbox`, oldest publication first so newer surveys win.
fn usgs_urls(bbox: [f64; 4], scratch: &Path) -> Result<Vec<String>> {
    let mut items: Vec<(String, String)> = Vec::new();
    for page in 0..10 {
        let path = scratch.join(format!("tnm-{page}.json"));
        http::discover_page(bbox, 100, page * 100, &path)?;
        let body: Value =
            serde_json::from_slice(&io_context("read TNM page", std::fs::read(&path))?)?;
        let list = body["items"]
            .as_array()
            .ok_or_else(|| AcquisitionError::Invalid("TNM response has no items".into()))?;
        for item in list {
            if let (Some(url), Some(date)) = (
                item["downloadURL"].as_str(),
                item["publicationDate"].as_str(),
            ) && url.to_ascii_lowercase().ends_with(".tif")
            {
                http::validate_url(url)?;
                items.push((date.to_owned(), url.to_owned()));
            }
        }
        if list.len() < 100 || body["total"].as_u64().unwrap_or(0) <= u64::from(page + 1) * 100 {
            break;
        }
    }
    items.sort();
    items.dedup_by(|a, b| a.1 == b.1);
    Ok(items.into_iter().map(|(_, url)| url).collect())
}

/// Average every public 1 m DEM under `bbox` onto the grid `extent` (in `srs`) at
/// `spacing_m`, reading the overview one level finer than the target. Returns Float32 LE
/// heights with NaN where no survey has data. Source tiles are staged under `cache`.
pub(crate) fn reduce(
    srs: &str,
    extent: [f64; 4],
    spacing_m: f64,
    bbox: [f64; 4],
    cache: &Path,
    scratch: &Path,
) -> Result<(Vec<u8>, Value)> {
    let usgs = usgs_urls(bbox, scratch)?;
    let (linz_urls, linz_collections) = linz::dem_urls(bbox, cache)?;
    let urls: Vec<&String> = usgs.iter().chain(&linz_urls).collect();
    ensure(
        !urls.is_empty(),
        "no public 1 m lidar DEM covers this region",
    )?;
    let placed = scratch.join("lidar.tif");
    let width = ((extent[2] - extent[0]) / spacing_m).round() as usize;
    let height = ((extent[3] - extent[1]) / spacing_m).round() as usize;
    run(super::cover::gdal("gdal_create")
        .args([
            "-q", "-of", "GTiff", "-ot", "Float32", "-bands", "1", "-burn", "nan",
        ])
        .args(["-a_nodata", "nan", "-a_srs", srs, "-a_ullr"])
        .args([extent[0], extent[3], extent[2], extent[1]].map(|v| v.to_string()))
        .args(["-outsize", &width.to_string(), &height.to_string()])
        .arg(&placed))?;
    // Tiles are downloaded whole (COG block reads over HTTP are latency bound), a few at
    // a time since each connection is slow, then averaged into the grid oldest first so
    // newer surveys replace older cells where they have data.
    let staging = io_context("create lidar staging", tempfile::tempdir_in(cache))?;
    for (n, chunk) in urls.chunks(PARALLEL_DOWNLOADS).enumerate() {
        eprintln!(
            "  sources {}-{} of {}",
            n * PARALLEL_DOWNLOADS + 1,
            n * PARALLEL_DOWNLOADS + chunk.len(),
            urls.len()
        );
        let files: Vec<_> = (0..chunk.len())
            .map(|i| staging.path().join(format!("source-{i}.tif")))
            .collect();
        let fetched: Vec<Result<()>> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .zip(&files)
                .map(|(url, file)| scope.spawn(move || http::fetch(url, file)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(AcquisitionError::Invalid("download thread panicked".into()))
                    })
                })
                .collect()
        });
        for ((url, file), result) in chunk.iter().zip(&files).zip(fetched) {
            let warp = |driver: &[&str]| {
                run(super::cover::gdal("gdalwarp")
                    .env("GDAL_CACHEMAX", "512")
                    .arg("-q")
                    .args(driver)
                    .args(["-ovr", "AUTO-1", "-r", "average", "-wm", "512"])
                    .arg(file)
                    .arg(&placed))
                .map(|_| ())
            };
            // GDAL's own TIFF reader has the LERC codec LINZ uses (some system libtiff builds
            // lack it) but crashes on a few USGS tiles that the libtiff driver reads fine.
            let result = result.and_then(|()| warp(&["-if", "LIBERTIFF"]).or_else(|_| warp(&[])));
            let _ = std::fs::remove_file(file);
            result.map_err(|e| AcquisitionError::Invalid(format!("{url}: {e}")))?;
        }
    }
    let details = json!({"usgs_urls": usgs, "linz_urls": linz_urls, "linz_collections": linz_collections,
        "resampling": "average from the overview one level finer than the target (gdalwarp -ovr AUTO-1); newer surveys drawn over older",
        "vertical_datum": "as published (NAVD88 for USGS 3DEP, NZVD2016 for LINZ)",
        "attribution": ["U.S. Geological Survey, 3D Elevation Program",
            "Toitū Te Whenua Land Information New Zealand (LINZ) and the collection's credited providers (CC BY 4.0)"]});
    Ok((
        review::band(&placed, scratch, "lidar", width * height, true)?,
        details,
    ))
}
