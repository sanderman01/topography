//! 16-bit heightmap texture tiles with quadtree LOD levels (master plan §5, §10).

use crate::acquisition::{
    AcquisitionError, Result, digest, ensure, io_context, raster, read_small, write_new,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq)]
pub enum Layout {
    SharedEdge,
    PowerOfTwo,
}

impl Layout {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "shared_edge" => Some(Self::SharedEdge),
            "power_of_two" => Some(Self::PowerOfTwo),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::SharedEdge => "shared_edge",
            Self::PowerOfTwo => "power_of_two",
        }
    }
}

fn read_layer(
    dir: &Path,
    name: &str,
    expected: &Value,
    cells: usize,
    size: usize,
) -> Result<Vec<u8>> {
    let bytes = io_context(format!("read {name}"), fs::read(dir.join(name)))?;
    ensure(
        bytes.len() == cells * size && expected[name].as_str() == Some(digest(&bytes).as_str()),
        format!("{name} does not match the world manifest"),
    )?;
    Ok(bytes)
}

fn png(
    path: &Path,
    width: usize,
    height: usize,
    depth: png::BitDepth,
    data: &[u8],
) -> Result<String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(depth);
        encoder.set_compression(png::Compression::High);
        let mut writer = encoder
            .write_header()
            .map_err(|e| AcquisitionError::Invalid(format!("png: {e}")))?;
        writer
            .write_image_data(data)
            .map_err(|e| AcquisitionError::Invalid(format!("png: {e}")))?;
    }
    write_new(path, &out)?;
    Ok(digest(&out))
}

fn geotiff(
    stage: &Path,
    name: &str,
    data: &[u8],
    width: usize,
    float: bool,
    georef: &[String],
) -> Result<()> {
    let height = data.len() / width / if float { 4 } else { 1 };
    let raw = stage.join(format!(".{name}.raw"));
    fs::write(&raw, data).map_err(|e| AcquisitionError::Invalid(format!("write raw: {e}")))?;
    let header = format!(
        "ENVI\nsamples = {width}\nlines = {height}\nbands = 1\nheader offset = 0\ndata type = {}\ninterleave = bsq\nbyte order = 0\n",
        if float { 4 } else { 1 }
    );
    fs::write(raw.with_extension("hdr"), header)
        .map_err(|e| AcquisitionError::Invalid(format!("write header: {e}")))?;
    let result = raster::run(
        raster::command("gdal_translate")
            .env("GDAL_DISABLE_READDIR_ON_OPEN", "FALSE")
            .args(["-q", "-of", "COG", "-co", "COMPRESS=DEFLATE"])
            .args(if float {
                ["-co", "PREDICTOR=YES"]
            } else {
                ["-co", "PREDICTOR=NO"]
            })
            .args(georef)
            .arg(&raw)
            .arg(stage.join(format!("{name}.tif"))),
    );
    let _ = fs::remove_file(&raw);
    let _ = fs::remove_file(raw.with_extension("hdr"));
    result.map(|_| ())
}

/// Export a generated world as 16-bit height tiles, 8-bit mask tiles and COG masters.
pub fn heightmap(world: &Path, output: &Path, intervals: usize, layout: Layout) -> Result<PathBuf> {
    ensure(!output.exists(), "export output already exists")?;
    ensure(
        intervals.is_power_of_two() && (16..=4096).contains(&intervals),
        "intervals must be a power of two from 16 to 4096",
    )?;
    let manifest: Value = serde_json::from_slice(&read_small(&world.join("manifest.json"))?)?;
    // Full generation is 1 m; quick scaled runs and previews are coarser.
    let spacing = manifest["spacing_m"].as_f64().unwrap_or(0.0);
    ensure(
        (manifest["kind"] == "diffusion_world_output" || manifest["kind"] == "world_preview")
            && spacing >= 1.0,
        "unsupported world output",
    )?;
    let input_dir = PathBuf::from(manifest["world_input"].as_str().unwrap_or_default());
    let input: Value = serde_json::from_slice(&read_small(&input_dir.join("world.json"))?)?;
    let [w, h]: [usize; 2] = serde_json::from_value(manifest["size"].clone())?;
    ensure(
        w == h && w.is_power_of_two() && w >= intervals,
        "world must be a power-of-two square of at least one tile",
    )?;
    let cells = w * h;
    let layers = &manifest["layers"];
    let elevation: Vec<f32> = read_layer(world, "elevation.f32", layers, cells, 4)?
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let water = read_layer(world, "water.u8", layers, cells, 1)?;
    let mut masks = vec![
        (
            "water",
            water
                .iter()
                .map(|&v| if v == 255 { 255 } else { 0 })
                .collect::<Vec<u8>>(),
        ),
        (
            "snow-ice",
            read_layer(world, "snow-ice.u8", layers, cells, 1)?
                .iter()
                .map(|&v| if v == 2 { 255 } else { 0 })
                .collect(),
        ),
        (
            "vegetation",
            read_layer(world, "vegetation.u8", layers, cells, 1)?
                .iter()
                .map(|&v| if v == 2 { 255 } else { 0 })
                .collect(),
        ),
    ];
    // Categorical WorldCover classes, exported as-is for render materials.
    let land_cover = layers.get("land-cover.u8").is_some();
    if land_cover {
        masks.push((
            "land-cover",
            read_layer(world, "land-cover.u8", layers, cells, 1)?,
        ));
    }
    // Range from land only; water fill is clamped and counted.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (e, wv) in elevation.iter().zip(&water) {
        ensure(e.is_finite(), "non-finite elevation")?;
        if *wv != 255 {
            lo = lo.min(*e);
            hi = hi.max(*e);
        }
    }
    ensure(hi > lo, "world needs land with a positive height range")?;
    let (h_min, h_max) = (f64::from(lo), f64::from(hi));
    let step = (h_max - h_min) / 65535.0;
    let mut clamped = 0usize;
    let codes: Vec<u16> = elevation
        .iter()
        .map(|&e| {
            let c = ((f64::from(e) - h_min) / step).round();
            if !(0.0..=65535.0).contains(&c) {
                clamped += 1;
            }
            c.clamp(0.0, 65535.0) as u16
        })
        .collect();
    let mut max_error = 0f64;
    for ((e, c), wv) in elevation.iter().zip(&codes).zip(&water) {
        if *wv != 255 {
            max_error = max_error.max((h_min + f64::from(*c) * step - f64::from(*e)).abs());
        }
    }

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create export parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage export", tempfile::tempdir_in(parent))?;
    let mut tiles = Vec::new();
    let mut levels = Vec::new();
    let mut level = 0u32;
    loop {
        let stride = 1usize << level;
        let samples = w / stride; // samples per side on this level's lattice
        let per_tile = intervals + usize::from(layout == Layout::SharedEdge);
        let count = samples.div_ceil(intervals);
        for ty in 0..count {
            for tx in 0..count {
                // Tiles stop at the world edge; the last shared-edge tile has no closing sample.
                let (x0, y0) = (tx * intervals, ty * intervals);
                let tw = per_tile.min(samples - x0);
                let th = per_tile.min(samples - y0);
                let mut height_px = Vec::with_capacity(tw * th * 2);
                let mut mask_px = vec![Vec::with_capacity(tw * th); masks.len()];
                for r in 0..th {
                    for c in 0..tw {
                        let i = (y0 + r) * stride * w + (x0 + c) * stride;
                        height_px.extend_from_slice(&codes[i].to_be_bytes());
                        for (m, (_, values)) in mask_px.iter_mut().zip(&masks) {
                            m.push(values[i]);
                        }
                    }
                }
                let name = format!("L{level}/{tx}_{ty}.png");
                let mut hashes = serde_json::Map::new();
                hashes.insert(
                    "height".into(),
                    json!(png(
                        &stage.path().join("height").join(&name),
                        tw,
                        th,
                        png::BitDepth::Sixteen,
                        &height_px
                    )?),
                );
                for (m, (layer, _)) in mask_px.iter().zip(&masks) {
                    hashes.insert(
                        (*layer).into(),
                        json!(png(
                            &stage.path().join(layer).join(&name),
                            tw,
                            th,
                            png::BitDepth::Eight,
                            m
                        )?),
                    );
                }
                tiles.push(json!({"level": level, "x": tx, "y": ty, "width": tw, "height": th, "sha256": hashes}));
            }
        }
        levels.push(json!({"level": level, "spacing_m": stride as f64 * spacing, "tiles_per_side": count, "samples_per_side": samples}));
        if samples <= intervals {
            break;
        }
        level += 1;
    }

    let origin: [f64; 2] = serde_json::from_value(input["origin"].clone())?;
    let crs = input["crs"].as_str().unwrap_or_default().to_owned();
    let georef = vec![
        "-a_srs".into(),
        crs.clone(),
        "-a_ullr".into(),
        origin[0].to_string(),
        origin[1].to_string(),
        (origin[0] + w as f64 * spacing).to_string(),
        (origin[1] - h as f64 * spacing).to_string(),
    ];
    let bytes: Vec<u8> = elevation.iter().flat_map(|e| e.to_le_bytes()).collect();
    geotiff(stage.path(), "elevation", &bytes, w, true, &georef)?;
    for (layer, values) in &masks {
        geotiff(stage.path(), layer, values, w, false, &georef)?;
    }
    let result = json!({
        "schema_version": 1,
        "kind": "heightmap_export",
        "world_output": world.canonicalize().map(|p| p.display().to_string()).unwrap_or_default(),
        "world_output_manifest_sha256": digest(&read_small(&world.join("manifest.json"))?),
        "crs": crs,
        "origin_nw_corner": origin,
        "spacing_m": spacing,
        "size": [w, h],
        "row_order": "north to south; texel (0,0) is the north-west cell centre",
        "layout": layout.name(),
        "intervals": intervals,
        "lod": "nested decimation: level k texel = level-0 sample at index * 2^k",
        "encoding": {"format": "PNG grayscale 16-bit", "h_min_m": h_min, "h_max_m": h_max, "step_m": step,
            "decode": "height_m = h_min_m + code * step_m", "range_from": "land cells (water fill excluded)",
            "clamped_cells": clamped, "max_land_quantization_error_m": max_error},
        "masks": {"format": "PNG grayscale 8-bit, 255 present, 0 absent; land-cover holds ESA WorldCover class codes",
            "layers": masks.iter().map(|(name, _)| *name).collect::<Vec<_>>()},
        "masters": ["elevation.tif (Float32 m COG)", "water.tif", "snow-ice.tif", "vegetation.tif", "land-cover.tif (when present)"],
        "attribution": [&input["base"]["attribution"], &input["cover"]["attribution"]],
        "levels": levels,
        "tiles": tiles,
    });
    write_new(
        &stage.path().join("manifest.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    io_context("publish export", fs::rename(stage.path(), output))?;
    Ok(output.join("manifest.json"))
}
