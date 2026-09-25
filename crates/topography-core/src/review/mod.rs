//! Terrain quality metrics, preview assets, and human review checkpoints.

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
