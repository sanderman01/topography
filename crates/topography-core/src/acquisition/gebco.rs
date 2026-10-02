//! GEBCO global grid (15 arc-second) read over HTTPS as a coarse seabed under GLO-30 sea.

use super::{Result, http, raster::run};
use crate::review;
use serde_json::{Value, json};
use std::path::Path;

const URL: &str =
    "https://dap.ceda.ac.uk/bodc/gebco/global/gebco_2025/ice_surface_elevation/geotiff";
pub(crate) const ATTRIBUTION: &str = "GEBCO Compilation Group (2025) GEBCO 2025 Grid";

/// The 90 x 90 degree tiles GEBCO publishes, as their south-west corners.
fn tiles(bbox: [f64; 4]) -> Vec<String> {
    let corner = |v: f64, step: f64| (v / step).floor() * step;
    let mut out = Vec::new();
    for s in [corner(bbox[1], 90.0), corner(bbox[3], 90.0)] {
        for w in [corner(bbox[0], 90.0), corner(bbox[2], 90.0)] {
            let name = format!(
                "{URL}/gebco_2025_n{:.1}_s{:.1}_w{:.1}_e{:.1}.tif",
                s + 90.0,
                s,
                w,
                w + 90.0
            );
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// GEBCO heights on a metric grid, smoothly resampled (only the seabed is used, and its
/// detail is not a goal). `extent` is [xmin, ymin, xmax, ymax] in `srs`.
pub(crate) fn seabed(
    srs: &str,
    extent: [f64; 4],
    spacing_m: f64,
    bbox: [f64; 4],
    scratch: &Path,
) -> Result<(Vec<u8>, Value)> {
    let urls = tiles(bbox);
    for url in &urls {
        super::ensure(http::exists(url)?, format!("GEBCO tile missing: {url}"))?;
    }
    let vrt = scratch.join("gebco.vrt");
    run(super::cover::gdal("gdalbuildvrt")
        .arg("-q")
        .arg(&vrt)
        .args(urls.iter().map(|u| format!("/vsicurl/{u}"))))?;
    let placed = scratch.join("gebco.tif");
    run(super::cover::gdal("gdalwarp")
        .args(["-q", "-r", "cubicspline", "-ot", "Float32"])
        .args(["-t_srs", srs, "-te"])
        .args(extent.map(|v| v.to_string()))
        .args(["-tr", &spacing_m.to_string(), &spacing_m.to_string()])
        .args(["-of", "GTiff"])
        .arg(&vrt)
        .arg(&placed))?;
    let width = ((extent[2] - extent[0]) / spacing_m).round() as usize;
    let height = ((extent[3] - extent[1]) / spacing_m).round() as usize;
    let details = json!({"urls": urls, "attribution": ATTRIBUTION, "resampling": "cubicspline",
        "note": "ice-surface elevation, about 450 m; used below sea level only"});
    Ok((
        review::band(&placed, scratch, "gebco", width * height, true)?,
        details,
    ))
}
