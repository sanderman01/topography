//! 16-bit heightmap texture tiles with quadtree LOD levels (master plan §5, §10).

use crate::acquisition::{
    AcquisitionError, Result, digest, ensure, hash_file, io_context, raster, read_small, write_new,
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

/// One raw world layer, verified against the manifest and read a few rows at a time.
struct Layer {
    name: &'static str,
    file: fs::File,
    cell: usize,
    width: usize,
    map: fn(u8) -> u8,
}

impl Layer {
    fn open(
        dir: &Path,
        name: &'static str,
        expected: &Value,
        (width, height): (usize, usize),
        cell: usize,
        map: fn(u8) -> u8,
    ) -> Result<Self> {
        let path = dir.join(name);
        let (sha, len) = hash_file(&path)?;
        ensure(
            len == (width * height * cell) as u64 && expected[name].as_str() == Some(sha.as_str()),
            format!("{name} does not match the world manifest"),
        )?;
        let file = io_context(format!("open {name}"), fs::File::open(&path))?;
        Ok(Self {
            name,
            file,
            cell,
            width,
            map,
        })
    }

    /// Raw bytes of `count` rows starting at row `y`.
    fn rows(&mut self, y: usize, count: usize, buf: &mut Vec<u8>) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        buf.resize(count * self.width * self.cell, 0);
        let offset = (y * self.width * self.cell) as u64;
        io_context(
            format!("read {}", self.name),
            self.file
                .seek(SeekFrom::Start(offset))
                .and_then(|_| self.file.read_exact(buf)),
        )
    }
}

fn f32_at(bytes: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([
        bytes[4 * i],
        bytes[4 * i + 1],
        bytes[4 * i + 2],
        bytes[4 * i + 3],
    ])
}

/// Rows per read when streaming whole layers.
const CHUNK_ROWS: usize = 256;

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
    layer: &mut Layer,
    out_name: &str,
    height: usize,
    georef: &[String],
) -> Result<()> {
    use std::io::Write;
    let float = layer.cell == 4;
    let width = layer.width;
    let raw = stage.join(format!(".{out_name}.raw"));
    let mut writer = std::io::BufWriter::new(io_context("create raw", fs::File::create(&raw))?);
    let mut buf = Vec::new();
    for y in (0..height).step_by(CHUNK_ROWS) {
        let count = CHUNK_ROWS.min(height - y);
        layer.rows(y, count, &mut buf)?;
        if !float {
            buf.iter_mut().for_each(|v| *v = (layer.map)(*v));
        }
        io_context("write raw", writer.write_all(&buf))?;
    }
    io_context("flush raw", writer.flush())?;
    drop(writer);
    let header = format!(
        "ENVI\nsamples = {width}\nlines = {height}\nbands = 1\nheader offset = 0\ndata type = {}\ninterleave = bsq\nbyte order = 0\n",
        if float { 4 } else { 1 }
    );
    fs::write(raw.with_extension("hdr"), header)
        .map_err(|e| AcquisitionError::Invalid(format!("write header: {e}")))?;
    let result = raster::run(
        raster::command("gdal_translate")
            .env("GDAL_DISABLE_READDIR_ON_OPEN", "FALSE")
            .args([
                "-q",
                "-of",
                "COG",
                "-co",
                "COMPRESS=DEFLATE",
                "-co",
                "BIGTIFF=IF_SAFER",
            ])
            .args(if float {
                ["-co", "PREDICTOR=YES"]
            } else {
                ["-co", "PREDICTOR=NO"]
            })
            .args(georef)
            .arg(&raw)
            .arg(stage.join(format!("{out_name}.tif"))),
    );
    let _ = fs::remove_file(&raw);
    let _ = fs::remove_file(raw.with_extension("hdr"));
    result.map(|_| ())
}

/// Export a generated world as 16-bit height tiles and 8-bit mask tiles; with `masters`,
/// also full-world COGs (Float32 elevation and the masks), which duplicate the world output.
pub fn heightmap(
    world: &Path,
    output: &Path,
    intervals: usize,
    layout: Layout,
    masters: bool,
) -> Result<PathBuf> {
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
    let layers = &manifest["layers"];
    let mut elevation = Layer::open(world, "elevation.f32", layers, (w, h), 4, |v| v)?;
    let mut masks = vec![
        Layer::open(world, "water.u8", layers, (w, h), 1, |v| {
            if v == 255 { 255 } else { 0 }
        })?,
        Layer::open(world, "snow-ice.u8", layers, (w, h), 1, |v| {
            if v == 2 { 255 } else { 0 }
        })?,
        Layer::open(world, "vegetation.u8", layers, (w, h), 1, |v| {
            if v == 2 { 255 } else { 0 }
        })?,
    ];
    // Categorical layers exported as-is: WorldCover classes for render materials, and the
    // terrain class (biome) of authored worlds.
    for name in ["land-cover.u8", "biome.u8"] {
        if layers.get(name).is_some() {
            masks.push(Layer::open(world, name, layers, (w, h), 1, |v| v)?);
        }
    }
    let mask_name = |layer: &Layer| layer.name.trim_end_matches(".u8");
    // Range from all ground: terrain continues under water, so the seabed is encoded too.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut buf = Vec::new();
    for y in (0..h).step_by(CHUNK_ROWS) {
        elevation.rows(y, CHUNK_ROWS.min(h - y), &mut buf)?;
        for i in 0..buf.len() / 4 {
            let e = f32_at(&buf, i);
            ensure(e.is_finite(), "non-finite elevation")?;
            lo = lo.min(e);
            hi = hi.max(e);
        }
    }
    ensure(hi > lo, "world needs a positive height range")?;
    let (h_min, h_max) = (f64::from(lo), f64::from(hi));
    let step = (h_max - h_min) / 65535.0;
    let code = |e: f32| (((f64::from(e) - h_min) / step).round()).clamp(0.0, 65535.0) as u16;
    let mut max_error = 0f64;

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
            // One row of tiles: its lattice rows, decimated to this level, read from disk.
            let y0 = ty * intervals;
            let th = per_tile.min(samples - y0);
            let mut codes = Vec::with_capacity(th * samples);
            let mut mask_rows = vec![Vec::with_capacity(th * samples); masks.len()];
            for r in 0..th {
                let y = (y0 + r) * stride;
                elevation.rows(y, 1, &mut buf)?;
                for x in 0..samples {
                    let e = f32_at(&buf, x * stride);
                    let c = code(e);
                    if level == 0 {
                        max_error =
                            max_error.max((h_min + f64::from(c) * step - f64::from(e)).abs());
                    }
                    codes.push(c);
                }
                for (layer, out) in masks.iter_mut().zip(&mut mask_rows) {
                    layer.rows(y, 1, &mut buf)?;
                    out.extend((0..samples).map(|x| (layer.map)(buf[x * stride])));
                }
            }
            for tx in 0..count {
                // Tiles stop at the world edge; the last shared-edge tile has no closing sample.
                let x0 = tx * intervals;
                let tw = per_tile.min(samples - x0);
                let mut height_px = Vec::with_capacity(tw * th * 2);
                let mut mask_px = vec![Vec::with_capacity(tw * th); masks.len()];
                for r in 0..th {
                    let row = r * samples + x0;
                    for c in &codes[row..row + tw] {
                        height_px.extend_from_slice(&c.to_be_bytes());
                    }
                    for (m, values) in mask_px.iter_mut().zip(&mask_rows) {
                        m.extend_from_slice(&values[row..row + tw]);
                    }
                }
                let mut hashes = serde_json::Map::new();
                hashes.insert(
                    "height".into(),
                    json!(png(
                        &stage.path().join(super::tile_path("height", level, tx, ty)),
                        tw,
                        th,
                        png::BitDepth::Sixteen,
                        &height_px
                    )?),
                );
                for (m, layer) in mask_px.iter().zip(&masks) {
                    let layer = mask_name(layer);
                    hashes.insert(
                        layer.into(),
                        json!(png(
                            &stage.path().join(super::tile_path(layer, level, tx, ty)),
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
    let mut master_files = Vec::new();
    if masters {
        geotiff(stage.path(), &mut elevation, "elevation", h, &georef)?;
        master_files.push("elevation.tif (Float32 m COG)".to_owned());
        for layer in &mut masks {
            let name = mask_name(layer);
            geotiff(stage.path(), layer, name, h, &georef)?;
            master_files.push(format!("{name}.tif"));
        }
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
            "decode": "height_m = h_min_m + code * step_m", "range_from": "all cells (ground continues under water)",
            "max_quantization_error_m": max_error},
        "masks": {"format": "PNG grayscale 8-bit, 255 present, 0 absent; land-cover holds ESA WorldCover class codes, biome the terrain class id (catalog/labels/terrain-v2.toml)",
            "layers": masks.iter().map(mask_name).collect::<Vec<_>>()},
        "tile_path": "{layer}/L{level}/{prefix}_L{level}_{x}_{y}.png; prefix: height, water, snow (snow-ice), veg (vegetation), landcover (land-cover), biome",
        "masters": master_files,
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
