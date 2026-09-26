//! Terrain quality metrics, preview assets, and human review checkpoints.

pub(crate) mod artifacts;
pub(crate) mod bundle;
pub mod gallery;
pub mod gallery_server;
mod overlay;
pub(crate) mod preparation_worker;

use crate::{
    acquisition::{
        Result, ensure, io_context,
        raster::{command, run},
        write_new,
    },
    catalog::source::SourceRecord,
};
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path};

// GDAL handles raster decoding. Only a bounded, single-band native crop is read here.
pub(crate) fn band(
    path: &Path,
    scratch: &Path,
    name: &str,
    cells: usize,
    float: bool,
) -> Result<Vec<u8>> {
    let output = scratch.join(format!("{name}.bin"));
    run(command("gdal_translate")
        // ENVI creation reopens its own header. Inputs here are owned review crops.
        .env("GDAL_DISABLE_READDIR_ON_OPEN", "TRUE")
        .args([
            "-q",
            "-of",
            "ENVI",
            "-ot",
            if float { "Float32" } else { "Byte" },
            "-co",
            "INTERLEAVE=BSQ",
            "-co",
            "SUFFIX=ADD",
        ])
        .arg(path)
        .arg(&output))?;
    let header = io_context(
        "read GDAL binary header",
        fs::read_to_string(output.with_extension("bin.hdr")),
    )?;
    let field = |key| {
        header.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then_some(v.trim())
        })
    };
    ensure(
        field("bands") == Some("1")
            && field("header offset") == Some("0")
            && field("data type") == Some(if float { "4" } else { "1" })
            && field("interleave") == Some("bsq"),
        "unexpected GDAL binary layout",
    )?;
    let order = field("byte order");
    ensure(matches!(order, Some("0" | "1")), "missing GDAL byte order")?;
    let length = cells * if float { 4 } else { 1 };
    let mut bytes = Vec::new();
    io_context(
        "read GDAL binary samples",
        fs::File::open(output)
            .and_then(|file| file.take(length as u64 + 1).read_to_end(&mut bytes)),
    )?;
    // GDAL's ENVI driver initializes a two-byte file, even for a one-byte band.
    ensure(
        bytes.len() == length || (!float && cells == 1 && bytes.len() == 2),
        "GDAL sample count mismatch",
    )?;
    bytes.truncate(length);
    if float && order == Some("1") {
        for value in bytes.chunks_exact_mut(4) {
            value.reverse();
        }
    }
    Ok(bytes)
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let a = group[0];
        let b = group.get(1).copied().unwrap_or(0);
        let c = group.get(2).copied().unwrap_or(0);
        text.push(ALPHABET[(a >> 2) as usize] as char);
        text.push(ALPHABET[((a & 3) << 4 | b >> 4) as usize] as char);
        text.push(if group.len() > 1 {
            ALPHABET[((b & 15) << 2 | c >> 6) as usize] as char
        } else {
            '='
        });
        text.push(if group.len() > 2 {
            ALPHABET[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    text
}

fn metre_unit(value: &str) -> bool {
    matches!(value, "m" | "metre" | "metres" | "meter" | "meters")
}

pub(crate) fn metric_grid(info: &Value, source: &SourceRecord, transform: &[f64]) -> bool {
    let proj = &info["stac"]["proj:projjson"];
    let horizontal = if proj["type"] == "CompoundCRS" {
        &proj["components"][0]
    } else {
        proj
    };
    let axes = horizontal["coordinate_system"]["axis"].as_array();
    let mapping = info["coordinateSystem"]["dataAxisToSRSAxisMapping"].as_array();
    let units = [
        source.spatial.horizontal_units.as_deref(),
        source.spatial.vertical_units.as_deref(),
    ];
    units.iter().all(|u| u.is_some_and(metre_unit))
        && info["bands"][0]["unit"].as_str().is_none_or(metre_unit)
        && horizontal["type"] == "ProjectedCRS"
        && axes.is_some_and(|axes| {
            axes.len() == 2
                && axes
                    .iter()
                    .all(|axis| axis["unit"].as_str().is_some_and(metre_unit))
                && mapping.is_some_and(|map| {
                    let x = map.first().and_then(Value::as_u64);
                    let y = map.get(1).and_then(Value::as_u64);
                    // CRS authority axis order can differ from raster x/y order.
                    match (x, y) {
                        (Some(1), Some(2)) => {
                            axes[0]["direction"] == "east" && axes[1]["direction"] == "north"
                        }
                        (Some(2), Some(1)) => {
                            axes[1]["direction"] == "east" && axes[0]["direction"] == "north"
                        }
                        _ => false,
                    }
                })
        })
        && transform[1] > 0.0
        && transform[5] < 0.0
        && transform[2] == 0.0
        && transform[4] == 0.0
}

/// WGS84 centre and corners (degrees, 5 decimals) of a review crop from its
/// georeferenced elevation, plus a Google Earth view about 2.5 tile widths away.
pub(crate) fn location(review: &Path, bundle: &bundle::Bundle) -> Result<Value> {
    // The verified inspection is gdalinfo of the same crop; older ones may lack corners.
    let info = if bundle.inspection["wgs84Extent"].is_object() {
        bundle.inspection.clone()
    } else {
        let elevation = review
            .parent()
            .unwrap_or(Path::new("."))
            .join("elevation.tif");
        serde_json::from_slice(&run(command("gdalinfo")
            .args(["-json", "-nomd"])
            .arg(&elevation))?)?
    };
    // gdalinfo lists the transformed corners as upper left, lower left, lower right, upper right.
    let ring: Vec<[f64; 2]> = info["wgs84Extent"]["coordinates"][0]
        .as_array()
        .into_iter()
        .flatten()
        .take(4)
        .filter_map(|p| Some([p[1].as_f64()?, p[0].as_f64()?]))
        .collect();
    ensure(ring.len() == 4, "review crop has no WGS84 corners")?;
    let round = |v: f64| (v * 1e5).round() / 1e5;
    let point = |p: [f64; 2]| json!([round(p[0]), round(p[1])]);
    let centre = [
        ring.iter().map(|p| p[0]).sum::<f64>() / 4.0,
        ring.iter().map(|p| p[1]).sum::<f64>() / 4.0,
    ];
    let transform: Vec<f64> =
        serde_json::from_value(bundle.inspection["geoTransform"].clone()).unwrap_or_default();
    let width_m = bundle.manifest.window_cells[2] as f64 * transform.get(1).copied().unwrap_or(1.0);
    let metrics = bundle.metrics.as_ref();
    let ground = match metrics.map(|m| (m["minimum_native"].as_f64(), m["maximum_native"].as_f64()))
    {
        Some((Some(low), Some(high))) => ((low + high) / 2.0).round(),
        _ => 0.0,
    };
    let url = format!(
        "https://earth.google.com/web/@{:.5},{:.5},{ground}a,{:.0}d,35y,0h,60t,0r",
        centre[0],
        centre[1],
        width_m * 2.5
    );
    Ok(json!({"centre": point(centre),
        "corners": {"nw": point(ring[0]), "sw": point(ring[1]), "se": point(ring[2]), "ne": point(ring[3])},
        "width_m": width_m, "google_earth": url}))
}

/// Native samples and provider-mask counts for inspection, never admission.
/// The browser uses a CPU heightfield preview; no model or GPU job is launched.
pub(crate) fn write_preview(directory: &Path, info: &Value, source: &SourceRecord) -> Result<()> {
    let width = info["size"][0].as_u64().unwrap_or(0) as usize;
    let height = info["size"][1].as_u64().unwrap_or(0) as usize;
    ensure(
        (1..=2048).contains(&width) && (1..=2048).contains(&height),
        "preview dimensions exceed crop limit",
    )?;
    let cells = width * height;
    let transform: Vec<f64> = info["geoTransform"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    ensure(
        transform.len() == 6 && transform.iter().all(|n| n.is_finite()),
        "preview needs a finite geotransform",
    )?;
    let scratch = io_context("stage preview samples", tempfile::tempdir_in(directory))?;
    let heights = band(
        &directory.join("elevation.tif"),
        scratch.path(),
        "heights",
        cells,
        true,
    )?;
    let mask = band(
        &directory.join("validity.tif"),
        scratch.path(),
        "mask",
        cells,
        false,
    )?;
    ensure(
        mask.iter().all(|m| matches!(m, 0 | 255)),
        "preview requires binary provider validity",
    )?;
    let mut finite_valid = 0usize;
    let mut nonfinite_valid = 0usize;
    let mut minimum = f32::INFINITY;
    let mut maximum = f32::NEG_INFINITY;
    for (bytes, valid) in heights.chunks_exact(4).zip(&mask) {
        let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if *valid != 0 {
            if value.is_finite() {
                finite_valid += 1;
                minimum = minimum.min(value);
                maximum = maximum.max(value);
            } else {
                nonfinite_valid += 1;
            }
        }
    }
    let physical_view = metric_grid(info, source, &transform);
    let metrics = json!({"schema_version": 1, "kind": "native_crop_inspection", "cells": cells,
        "provider_valid_cells": finite_valid + nonfinite_valid, "finite_valid_cells": finite_valid,
        "nonfinite_valid_cells": nonfinite_valid, "provider_invalid_cells": cells - finite_valid - nonfinite_valid,
        "minimum_native": (finite_valid > 0).then_some(minimum), "maximum_native": (finite_valid > 0).then_some(maximum),
        "vertical_units_declared": source.spatial.vertical_units, "vertical_datum_declared": source.spatial.vertical_datum,
        "metric_north_up_preview": physical_view, "review_state": "unreviewed",
        "vehicle_envelope": null, "terrestrial_coverage": null, "admission": "not evaluated"});
    write_new(
        &directory.join("metrics.json"),
        &serde_json::to_vec_pretty(&metrics)?,
    )?;
    let mut shades = Vec::new();
    for azimuth in [315, 45, 225] {
        let path = scratch.path().join(format!("shade-{azimuth}.tif"));
        run(command("gdaldem")
            .args([
                "hillshade",
                "-q",
                "-compute_edges",
                "-az",
                &azimuth.to_string(),
            ])
            .arg(directory.join("elevation.tif"))
            .arg(&path))?;
        shades.push(base64(&band(
            &path,
            scratch.path(),
            &format!("shade-{azimuth}"),
            cells,
            false,
        )?));
    }
    let payload = json!({"width": width, "height": height, "transform": transform,
        "record_id": source.record_id, "metrics": metrics,
        "elevation_le_f32": base64(&heights), "mask": base64(&mask), "shades": shades});
    // Escaping '<' prevents retained metadata from terminating the script element.
    let encoded = serde_json::to_string(&payload)?.replace('<', "\\u003c");
    let html = include_str!("preview.html").replace("__TERRAIN_DATA__", &encoded);
    write_new(&directory.join("preview.html"), html.as_bytes())
}

/// Share of valid ground with slope under 3 degrees, measured on 4 m block means so
/// micro-relief does not count. Near 1 for floodplains, water and flat fields.
pub(crate) fn flat_share(review: &Path, bundle: &bundle::Bundle) -> Result<f64> {
    let [_, _, w, h] = bundle.manifest.window_cells;
    let (w, h) = (w as usize, h as usize);
    let dir = review.parent().unwrap_or(Path::new("."));
    let scratch = io_context("stage flatness check", tempfile::tempdir())?;
    let elevation = band(
        &dir.join("elevation.tif"),
        scratch.path(),
        "elevation",
        w * h,
        true,
    )?;
    let validity = band(
        &dir.join("validity.tif"),
        scratch.path(),
        "validity",
        w * h,
        false,
    )?;
    let transform: Vec<f64> =
        serde_json::from_value(bundle.inspection["geoTransform"].clone()).unwrap_or_default();
    let spacing = transform.get(1).copied().unwrap_or(1.0).abs();
    let k = (4.0 / spacing).round().max(1.0) as usize;
    let step = k as f64 * spacing;
    let (bw, bh) = (w / k, h / k);
    let mut coarse = vec![f64::NAN; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let mut sum = 0.0;
            let mut ok = true;
            for y in by * k..(by + 1) * k {
                for x in bx * k..(bx + 1) * k {
                    let i = y * w + x;
                    let b = &elevation[i * 4..i * 4 + 4];
                    let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    ok &= validity[i] != 0 && v.is_finite();
                    sum += f64::from(v);
                }
            }
            if ok {
                coarse[by * bw + bx] = sum / (k * k) as f64;
            }
        }
    }
    let limit = 3f64.to_radians().tan() * 2.0 * step;
    let (mut flat, mut total) = (0usize, 0usize);
    for y in 1..bh.saturating_sub(1) {
        for x in 1..bw.saturating_sub(1) {
            let dx = coarse[y * bw + x + 1] - coarse[y * bw + x - 1];
            let dy = coarse[(y + 1) * bw + x] - coarse[(y - 1) * bw + x];
            if dx.is_finite() && dy.is_finite() {
                total += 1;
                flat += usize::from(dx.hypot(dy) < limit);
            }
        }
    }
    // Too small to measure is not evidence of flatness.
    Ok(if total == 0 {
        0.0
    } else {
        flat as f64 / total as f64
    })
}
