//! Copernicus GLO-30 public DEM read as a coarse world base.

use super::{Result, http, raster::run};
use crate::review;
use serde_json::{Value, json};
use std::path::Path;

const URL: &str = "https://copernicus-dem-30m.s3.amazonaws.com";
const ATTRIBUTION: &str = "Produced using Copernicus WorldDEM-30 © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved";

fn tile(lat: i64, lon: i64) -> String {
    let name = format!(
        "Copernicus_DSM_COG_10_{}{:02}_00_{}{:03}_00_DEM",
        if lat < 0 { 'S' } else { 'N' },
        lat.abs(),
        if lon < 0 { 'W' } else { 'E' },
        lon.abs()
    );
    format!("{URL}/{name}/{name}.tif")
}

/// Average GLO-30 elevation onto a metric grid. Cells without a published tile
/// (open ocean) are 0 m. `extent` is [xmin, ymin, xmax, ymax] in `srs`.
pub(crate) fn coarse(
    srs: &str,
    extent: [f64; 4],
    spacing_m: f64,
    bbox: [f64; 4],
    scratch: &Path,
) -> Result<(Vec<u8>, Value)> {
    let (mut urls, mut missing) = (Vec::new(), Vec::new());
    for lat in bbox[1].floor() as i64..=bbox[3].floor() as i64 {
        for lon in bbox[0].floor() as i64..=bbox[2].floor() as i64 {
            let url = tile(lat, lon);
            if http::exists(&url)? {
                urls.push(url);
            } else {
                missing.push(url);
            }
        }
    }
    let width = ((extent[2] - extent[0]) / spacing_m).round() as usize;
    let height = ((extent[3] - extent[1]) / spacing_m).round() as usize;
    let details = json!({"urls": urls, "missing_as_sea_level": missing, "attribution": ATTRIBUTION,
        "resampling": "average", "note": "surface model including canopy; judgment-grade base"});
    if urls.is_empty() {
        return Ok((vec![0; width * height * 4], details));
    }
    let vrt = scratch.join("glo30.vrt");
    run(super::cover::gdal("gdalbuildvrt")
        .arg("-q")
        .arg(&vrt)
        .args(urls.iter().map(|u| format!("/vsicurl/{u}"))))?;
    let placed = scratch.join("glo30.tif");
    run(super::cover::gdal("gdalwarp")
        .args([
            "-q",
            "-r",
            "average",
            "-ot",
            "Float32",
            "-wo",
            "INIT_DEST=0",
        ])
        .args(["-dstnodata", "None", "-t_srs", srs, "-te"])
        .args(extent.map(|v| v.to_string()))
        .args(["-tr", &spacing_m.to_string(), &spacing_m.to_string()])
        .args(["-of", "GTiff"])
        .arg(&vrt)
        .arg(&placed))?;
    Ok((
        review::band(&placed, scratch, "glo30", width * height, true)?,
        details,
    ))
}
