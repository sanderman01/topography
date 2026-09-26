use super::{
    AcquisitionError, Result, ensure, hash_file, io_context, read_small, store, write_new,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env("GDAL_PAM_ENABLED", "NO")
        .env("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR")
        .env("GDAL_GEOREF_SOURCES", "INTERNAL")
        .env("GDAL_CACHEMAX", "64")
        .env("GDAL_NUM_THREADS", "1");
    command
}

pub(crate) fn run(command: &mut Command) -> Result<Vec<u8>> {
    let output = super::bounded_output(command)?;
    ensure(
        output.status.success(),
        format!(
            "GDAL {} failed ({}): {}",
            command.get_program().to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    ensure(
        output.stdout.len() <= 1024 * 1024,
        "GDAL metadata exceeds 1 MiB",
    )?;
    Ok(output.stdout)
}

pub(super) fn inspect(path: &Path) -> Result<(Value, String)> {
    let absolute = io_context("resolve raster path", fs::canonicalize(path))?;
    let inspect_with = |driver| {
        run(command("gdalinfo")
            .args(["-json", "-noct", "-norat", "-nogcp", "-if", driver])
            .arg(&absolute))
    };
    // Some system libtiff builds lack codecs available in GDAL's read-only reader.
    let bytes = inspect_with("GTiff").or_else(|primary| {
        inspect_with("LIBERTIFF").map_err(|alternate| {
            AcquisitionError::Invalid(format!("{primary}; alternate TIFF reader: {alternate}"))
        })
    })?;
    let mut info: Value = serde_json::from_slice(&bytes)?;
    ensure(
        matches!(
            info["driverShortName"].as_str(),
            Some("GTiff" | "LIBERTIFF")
        ),
        "only self-contained GeoTIFF DEMs are supported",
    )?;
    ensure(
        info["size"].as_array().is_some_and(|a| {
            a.len() == 2
                && a.iter()
                    .all(|v| v.as_u64().is_some_and(|n| n > 0 && n <= i32::MAX as u64))
        }),
        "DEM has invalid raster dimensions",
    )?;
    let transform = info["geoTransform"]
        .as_array()
        .ok_or_else(|| AcquisitionError::Invalid("DEM lacks an internal geotransform".into()))?;
    ensure(
        transform.len() == 6
            && transform
                .iter()
                .all(|v| v.as_f64().is_some_and(f64::is_finite)),
        "invalid raster geotransform",
    )?;
    let value = |index: usize| transform[index].as_f64().unwrap_or(0.0);
    let determinant = value(1) * value(5) - value(2) * value(4);
    ensure(
        determinant.is_finite() && determinant != 0.0,
        "raster geotransform is singular",
    )?;
    ensure(
        info["coordinateSystem"]["wkt"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "DEM lacks an internal CRS",
    )?;
    ensure(
        info["bands"]
            .as_array()
            .is_some_and(|b| b.len() == 1 && b[0]["type"] == "Float32"),
        "initial DEM import requires one Float32 band; RGB, integer encodings and multiband rasters are unsupported",
    )?;
    // Remove run-local paths so identical imports have stable inspection checksums.
    info["description"] = json!("asset");
    info["files"] = json!(["asset"]);
    let version = String::from_utf8_lossy(&run(command("gdalinfo").arg("--version"))?)
        .trim()
        .to_string();
    Ok((info, version))
}

/// Extract an unscaled native-grid crop and validity mask for QGIS review.
/// This creates review artifacts only; source admission remains a separate gate.
pub fn create_review(receipt_path: &Path, output: &Path, window: [u32; 4]) -> Result<PathBuf> {
    let receipt = store::verify(receipt_path)?;
    let asset = store::asset_path(receipt_path, &receipt)?;
    let (info, gdal_version) = inspect(&asset)?;
    let reader = info["driverShortName"]
        .as_str()
        .ok_or_else(|| AcquisitionError::Invalid("missing inspected TIFF reader".into()))?;
    let [x, y, width, height] = window;
    ensure(
        width > 0 && height > 0 && width <= 2048 && height <= 2048,
        "review crops must be 1..=2048 cells on each side",
    )?;
    ensure(
        u64::from(x) + u64::from(width) <= info["size"][0].as_u64().unwrap_or(0)
            && u64::from(y) + u64::from(height) <= info["size"][1].as_u64().unwrap_or(0),
        "review window lies outside the raster",
    )?;
    ensure(
        !output.exists(),
        "review output already exists; choose a new directory",
    )?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create review parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage review", tempfile::tempdir_in(parent))?;
    let elevation = stage.path().join("elevation.tif");
    let mask = stage.path().join("validity.tif");
    let hillshade = stage.path().join("hillshade.tif");
    let window = window.map(|v| v.to_string());
    let asset = io_context("resolve raw asset", fs::canonicalize(asset))?;
    run(command("gdal_translate")
        .args(["-if", reader])
        .args(["-q", "-of", "GTiff", "-ot", "Float32", "-srcwin"])
        .args(&window)
        .args([
            "-mask",
            "auto",
            "-co",
            "TILED=YES",
            "-co",
            "COMPRESS=DEFLATE",
        ])
        .arg(&asset)
        .arg(&elevation))?;
    run(command("gdal_translate")
        .args([
            "-q",
            "-of",
            "GTiff",
            "-ot",
            "Byte",
            "-b",
            "mask",
            "-mask",
            "none",
            "-co",
            "TILED=YES",
            "-co",
            "COMPRESS=DEFLATE",
        ])
        .arg(&elevation)
        .arg(&mask))?;
    run(command("gdaldem")
        .args([
            "hillshade",
            "-q",
            "-compute_edges",
            "-co",
            "COMPRESS=DEFLATE",
        ])
        .arg(&elevation)
        .arg(&hillshade))?;
    let (crop_info, _) = inspect(&elevation)?;
    let source_path = asset.with_file_name(&receipt.source_sha256);
    let source: crate::catalog::source::SourceRecord =
        toml::from_str(&String::from_utf8_lossy(&read_small(&source_path)?))
            .map_err(|e| AcquisitionError::Invalid(format!("invalid retained source: {e}")))?;
    crate::review::write_preview(stage.path(), &crop_info, &source)?;
    write_new(
        &stage.path().join("inspection.json"),
        &serde_json::to_vec_pretty(&crop_info)?,
    )?;
    let mut artifacts = Vec::new();
    for name in [
        "elevation.tif",
        "validity.tif",
        "hillshade.tif",
        "inspection.json",
        "metrics.json",
        "preview.html",
    ] {
        let path = stage.path().join(name);
        let (sha256, bytes) = hash_file(&path)?;
        io_context(
            "sync review artifact",
            fs::File::open(&path).and_then(|f| f.sync_all()),
        )?;
        artifacts.push(json!({"path": name, "sha256": sha256, "bytes": bytes}));
    }
    let manifest = json!({"schema_version": 1, "kind": "native_grid_review", "review_state": "unreviewed",
        "raw_receipt_sha256": super::digest(&read_small(receipt_path)?), "asset_sha256": receipt.asset_sha256,
        "window_cells": [x, y, width, height], "resampling": "none", "gdal_version": gdal_version,
        "topography_version": env!("CARGO_PKG_VERSION"), "artifacts": artifacts,
        "validity": "provider validity, 0 invalid / 255 valid; water and terrestrial quality have not been classified",
        "admission": "raw review only; no prepared or training admission"});
    write_new(
        &stage.path().join("review.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    io_context("publish review", fs::rename(stage.path(), output))?;
    Ok(output.join("review.json"))
}

/// Project one lon/lat point into `srs` with gdaltransform.
pub(crate) fn project(srs: &str, lon: f64, lat: f64) -> Result<(f64, f64)> {
    let out = run(command("gdaltransform").args([
        "-s_srs",
        "EPSG:4326",
        "-t_srs",
        srs,
        "-output_xy",
        "-coord",
        &lon.to_string(),
        &lat.to_string(),
    ]))?;
    let text = String::from_utf8_lossy(&out);
    let mut parts = text.split_whitespace().map(str::parse::<f64>);
    match (parts.next(), parts.next()) {
        (Some(Ok(x)), Some(Ok(y))) => Ok((x, y)),
        _ => Err(super::AcquisitionError::Invalid(format!(
            "unexpected gdaltransform output: {text}"
        ))),
    }
}
