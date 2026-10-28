//! Heightmap and game layer texture tiles with area-mean quadtree LOD levels
//! (master plan §5, §10; manifest v2).

use super::layers::{APRON, GAME_CLASSES, GAME_DENSITIES, HEIGHT, LayerSpec, MOISTURE, metadata};
use super::pyramid::{Mean, Mode, Pyramid, Reduce};
use super::tiles::{Checks, Geometry, RowWindow, TileWriter, cut};
use crate::acquisition::{
    AcquisitionError, Result, digest, ensure, hash_file, io_context, raster, read_small,
};
use serde_json::{Map, Value, json};
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
    name: String,
    file: fs::File,
    cell: usize,
    width: usize,
}

impl Layer {
    fn open(
        dir: &Path,
        name: &str,
        expected: &Value,
        (width, height): (usize, usize),
        cell: usize,
    ) -> Result<Self> {
        let path = dir.join(name);
        let (sha, len) = hash_file(&path)?;
        ensure(
            len == (width * height * cell) as u64 && expected[name].as_str() == Some(sha.as_str()),
            format!("{name} does not match the world manifest"),
        )?;
        let file = io_context(format!("open {name}"), fs::File::open(&path))?;
        Ok(Self {
            name: name.into(),
            file,
            cell,
            width,
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

fn floats(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Rows per read when streaming whole layers.
const CHUNK_ROWS: usize = 256;

fn geotiff(
    stage: &Path,
    layer: &mut Layer,
    out_name: &str,
    height: usize,
    georef: &[String],
) -> Result<()> {
    use std::io::Write;
    let width = layer.width;
    let raw = stage.join(format!(".{out_name}.raw"));
    let mut writer = std::io::BufWriter::new(io_context("create raw", fs::File::create(&raw))?);
    let mut buf = Vec::new();
    for y in (0..height).step_by(CHUNK_ROWS) {
        layer.rows(y, CHUNK_ROWS.min(height - y), &mut buf)?;
        io_context("write raw", writer.write_all(&buf))?;
    }
    io_context("flush raw", writer.flush())?;
    drop(writer);
    let header = format!(
        "ENVI\nsamples = {width}\nlines = {height}\nbands = 1\nheader offset = 0\ndata type = 4\ninterleave = bsq\nbyte order = 0\n"
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
                "-co",
                "PREDICTOR=YES",
            ])
            .args(georef)
            .arg(&raw)
            .arg(stage.join(format!("{out_name}.tif"))),
    );
    let _ = fs::remove_file(&raw);
    let _ = fs::remove_file(raw.with_extension("hdr"));
    result.map(|_| ())
}

/// Shared state of the tile pass.
struct Ctx {
    writer: TileWriter,
    checks: Checks,
    geometry: Geometry,
    /// Level-0 samples per side.
    world: usize,
}

/// Cuts, encodes and writes tile row `ty` of one level; `rows` start at world row `first`.
#[allow(clippy::too_many_arguments)]
fn write_tiles<T: Copy>(
    ctx: &mut Ctx,
    spec: &LayerSpec,
    level: u32,
    ty: usize,
    first: usize,
    rows: &[Vec<T>],
    code: &dyn Fn(T) -> u16,
) -> Result<()> {
    let (g, a) = (ctx.geometry, spec.apron);
    let samples = ctx.world >> level;
    let (y0, th) = g.span(ty, samples);
    let h = th + 2 * a;
    let tile_rows: Vec<&[T]> = (0..h)
        .map(|r| rows[(y0 + r).saturating_sub(a).min(samples - 1) - first].as_slice())
        .collect();
    for tx in 0..g.count(samples) {
        let values = cut(&tile_rows, g, samples, tx, a);
        let texels: Vec<u8> = if spec.sixteen_bit {
            values.iter().flat_map(|&v| code(v).to_be_bytes()).collect()
        } else {
            values.iter().map(|&v| code(v) as u8).collect()
        };
        let w = values.len() / h;
        ctx.writer
            .write(spec, level, tx, ty, w, h, &texels, &mut ctx.checks)?;
    }
    Ok(())
}

/// One layer's row windows per level, fed by its pyramid's sink.
struct Output<'a, T: Clone> {
    spec: LayerSpec,
    windows: Vec<RowWindow<Vec<T>>>,
    code: &'a dyn Fn(T) -> u16,
}

impl<'a, T: Copy> Output<'a, T> {
    fn new(spec: LayerSpec, last: u32, ctx: &Ctx, code: &'a dyn Fn(T) -> u16) -> Self {
        let windows = (spec.finest_level..=last)
            .map(|l| RowWindow::new(ctx.geometry, ctx.world >> l, spec.apron))
            .collect();
        Self {
            spec,
            windows,
            code,
        }
    }

    fn row(&mut self, ctx: &mut Ctx, level: u32, y: usize, row: &[T]) -> Result<()> {
        let window = &mut self.windows[(level - self.spec.finest_level) as usize];
        match window.push(y, row.to_vec()) {
            Some((ty, first, rows)) => {
                write_tiles(ctx, &self.spec, level, ty, first, &rows, self.code)
            }
            None => Ok(()),
        }
    }
}

/// Game layers from `manifest["game"]`: their level and the class and density layers.
struct Game {
    level: u32,
    classes: Vec<(LayerSpec, Layer)>,
    densities: Vec<(LayerSpec, Layer)>,
}

fn open_game(world: &Path, manifest: &Value, w: usize) -> Result<Option<Game>> {
    let Some(game) = manifest.get("game") else {
        return Ok(None);
    };
    let level = game["level"].as_u64().unwrap_or(u64::MAX);
    ensure(level <= 1, "game layers must be at level 0 or 1")?;
    let level = level as u32;
    let m = w >> level;
    let size: [usize; 2] = serde_json::from_value(game["size"].clone())?;
    ensure(size == [m, m], "game layer size does not match the world")?;
    let names: Vec<String> = serde_json::from_value(game["layers"].clone())?;
    let mut out = Game {
        level,
        classes: Vec::new(),
        densities: Vec::new(),
    };
    for name in names {
        let open = |spec: &LayerSpec| -> Result<(LayerSpec, Layer)> {
            let file = format!("game/{name}.u8");
            let layer = Layer::open(world, &file, &manifest["layers"], (m, m), 1)?;
            Ok((
                LayerSpec {
                    finest_level: level,
                    ..*spec
                },
                layer,
            ))
        };
        if let Some(spec) = GAME_CLASSES.iter().find(|s| s.name == name) {
            out.classes.push(open(spec)?);
        } else if let Some(spec) = GAME_DENSITIES.iter().find(|s| s.name == name) {
            out.densities.push(open(spec)?);
        } else {
            return Err(AcquisitionError::Invalid(format!(
                "unknown game layer {name}"
            )));
        }
    }
    Ok(Some(out))
}

/// The first moisture level, built whole by bilinear sampling of the world's moisture grid
/// at texel positions; `None` without a moisture entry.
fn moisture(
    world: &Path,
    manifest: &Value,
    origin: [f64; 2],
    (w, spacing): (usize, f64),
    last: u32,
) -> Result<Option<(u32, Vec<Vec<f32>>)>> {
    let Some(entry) = manifest.get("moisture") else {
        eprintln!("world has no moisture entry; moisture tiles skipped");
        return Ok(None);
    };
    let [mw, mh]: [usize; 2] = serde_json::from_value(entry["size"].clone())?;
    let [nx, ny]: [f64; 2] = serde_json::from_value(entry["nw_m"].clone())?;
    let ms = entry["spacing_m"].as_f64().unwrap_or(0.0);
    let file = entry["file"].as_str().unwrap_or_default();
    ensure(mw > 0 && mh > 0 && ms > 0.0, "invalid world moisture entry")?;
    let mut layer = Layer::open(world, file, &manifest["layers"], (mw, mh), 4)?;
    let mut buf = Vec::new();
    layer.rows(0, mh, &mut buf)?;
    let grid: Vec<f32> = floats(&buf).collect();
    let level = (0..last)
        .find(|&k| spacing * f64::from(1u32 << k) >= 128.0)
        .unwrap_or(last);
    let (n, stride) = (w >> level, 1usize << level);
    // Texel i sits at level-0 sample i·2^k; sample centres are half a cell in.
    let at = |i: usize| (i * stride) as f64 * spacing + 0.5 * spacing;
    let rows = (0..n)
        .map(|i| {
            let fy = ((ny - (origin[1] - at(i))) / ms - 0.5).clamp(0.0, (mh - 1) as f64);
            (0..n)
                .map(|j| {
                    let fx = ((origin[0] + at(j) - nx) / ms - 0.5).clamp(0.0, (mw - 1) as f64);
                    let (x0, y0) = (fx as usize, fy as usize);
                    let (x1, y1) = ((x0 + 1).min(mw - 1), (y0 + 1).min(mh - 1));
                    let (ax, ay) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
                    let p = |x: usize, y: usize| grid[y * mw + x];
                    let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
                    let bottom = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
                    top * (1.0 - ay) + bottom * ay
                })
                .collect()
        })
        .collect();
    Ok(Some((level, rows)))
}

/// Dirs 0755 and files 0644 below `dir`: staged tempfiles are 0700/0600, and the game reads
/// exports as another user.
fn world_readable(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    io_context(
        "set export permissions",
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)),
    )?;
    for entry in io_context("list export", fs::read_dir(dir))? {
        let path = io_context("list export", entry)?.path();
        if path.is_dir() {
            world_readable(&path)?;
        } else {
            io_context(
                "set export permissions",
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)),
            )?;
        }
    }
    Ok(())
}

/// Export a generated world as 16-bit height tiles, game layer and moisture tiles with
/// aprons (manifest v2); with `masters`, also a Float32 elevation COG.
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
    let origin: [f64; 2] = serde_json::from_value(input["origin"].clone())?;
    let [w, h]: [usize; 2] = serde_json::from_value(manifest["size"].clone())?;
    ensure(
        w == h && w.is_power_of_two() && w >= intervals,
        "world must be a power-of-two square of at least one tile",
    )?;
    let layers = &manifest["layers"];
    ensure(
        ["water-surface.f32", "water-body.u32", "bodies.json"]
            .iter()
            .all(|name| layers.get(name).is_some() && world.join(name).is_file()),
        "world has no hydrology v2 water layers; regenerate it",
    )?;
    let mut elevation = Layer::open(world, "elevation.f32", layers, (w, h), 4)?;
    let mut surface = Layer::open(world, "water-surface.f32", layers, (w, h), 4)?;
    let mut game = open_game(world, &manifest, w)?;
    let mut last = 0u32;
    while (w >> last) > intervals {
        last += 1;
    }
    if let Some(game) = &game {
        ensure(
            game.level <= last,
            "game layers are coarser than the export's last level",
        )?;
    }
    let moisture = moisture(world, &manifest, origin, (w, spacing), last)?;

    // Range from ground and finite water surfaces: terrain continues under water.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut buf = Vec::new();
    for y in (0..h).step_by(CHUNK_ROWS) {
        let count = CHUNK_ROWS.min(h - y);
        elevation.rows(y, count, &mut buf)?;
        for e in floats(&buf) {
            ensure(e.is_finite(), "non-finite elevation")?;
            lo = lo.min(e);
            hi = hi.max(e);
        }
        surface.rows(y, count, &mut buf)?;
        for s in floats(&buf).filter(|s| s.is_finite()) {
            lo = lo.min(s);
            hi = hi.max(s);
        }
    }
    drop(surface);
    ensure(hi > lo, "world needs a positive height range")?;
    let (h_min, h_max) = (f64::from(lo), f64::from(hi));
    let step = (h_max - h_min) / 65535.0;
    let height_code = |e: f32| (((f64::from(e) - h_min) / step).round()).clamp(0.0, 65535.0) as u16;
    let class_code = |v: u8| u16::from(v);
    let density_code = |v: f32| v.round().clamp(0.0, 255.0) as u16;
    let fraction_code = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u16;
    let mut max_error = 0f64;

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create export parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage export", tempfile::tempdir_in(parent))?;
    let geometry = Geometry {
        intervals,
        shared: layout == Layout::SharedEdge,
    };
    let mut ctx = Ctx {
        writer: TileWriter::new(stage.path(), geometry.shared),
        checks: Checks::default(),
        geometry,
        world: w,
    };

    let height_spec = LayerSpec {
        scale: step,
        offset: h_min,
        ..HEIGHT
    };
    let mut height = (
        Pyramid::new(Mean, 0, last),
        Output::new(height_spec, last, &ctx, &height_code),
    );
    let (game_level, mut classes, mut densities) = match &mut game {
        Some(g) => (
            g.level,
            g.classes
                .iter_mut()
                .map(|(spec, layer)| {
                    let p = Pyramid::new(Mode, g.level, last);
                    (
                        p,
                        Output::new(*spec, last, &ctx, &class_code),
                        layer,
                        [0u64; 256],
                    )
                })
                .collect(),
            g.densities
                .iter_mut()
                .map(|(spec, layer)| {
                    let p = Pyramid::new(Mean, g.level, last);
                    (p, Output::new(*spec, last, &ctx, &density_code), layer)
                })
                .collect(),
        ),
        None => (0, Vec::new(), Vec::new()),
    };

    let mut game_buf = Vec::new();
    for y0 in (0..h).step_by(CHUNK_ROWS) {
        let count = CHUNK_ROWS.min(h - y0);
        elevation.rows(y0, count, &mut buf)?;
        for (r, bytes) in buf.chunks_exact(4 * w).enumerate() {
            let row: Vec<f32> = floats(bytes).collect();
            for &e in &row {
                let decoded = h_min + f64::from(height_code(e)) * step;
                max_error = max_error.max((decoded - f64::from(e)).abs());
            }
            let (p, out) = &mut height;
            p.push(row, &mut |l, y, row| out.row(&mut ctx, l, y, row))?;
            // A level-1 game row is complete once its level-0 rows 2i-1..2i+1 are.
            let y = y0 + r;
            let game_y = match game_level {
                0 => Some(y),
                _ => (y % 2 == 1).then_some(y / 2),
            };
            let Some(gy) = game_y else { continue };
            for (p, out, layer, counts) in &mut classes {
                layer.rows(gy, 1, &mut game_buf)?;
                for &v in &game_buf {
                    counts[usize::from(v)] += 1;
                }
                p.push(game_buf.clone(), &mut |l, y, row| {
                    out.row(&mut ctx, l, y, row)
                })?;
            }
            for (p, out, layer) in &mut densities {
                layer.rows(gy, 1, &mut game_buf)?;
                let row = game_buf.iter().map(|&v| f32::from(v)).collect();
                p.push(row, &mut |l, y, row| out.row(&mut ctx, l, y, row))?;
            }
        }
    }

    let mut layer_meta = Map::new();
    layer_meta.insert(
        height_spec.name.into(),
        metadata(&height_spec, spacing, last),
    );
    for (_, out, _, counts) in &classes {
        let mut m = metadata(&out.spec, spacing, last);
        // Classes present at the finest level, for the legends.
        m["present"] = json!((0..256).filter(|&c| counts[c] > 0).collect::<Vec<_>>());
        layer_meta.insert(out.spec.name.into(), m);
    }
    for (_, out, _) in &densities {
        layer_meta.insert(out.spec.name.into(), metadata(&out.spec, spacing, last));
    }
    if let Some((first, mut rows)) = moisture {
        let spec = LayerSpec {
            finest_level: first,
            ..MOISTURE
        };
        for level in first..=last {
            if level > first {
                let n = rows.len() / 2;
                rows = (0..n)
                    .map(|i| {
                        let above = &rows[(2 * i).saturating_sub(1)];
                        Mean.reduce([above, &rows[2 * i], &rows[2 * i + 1]])
                    })
                    .collect();
            }
            for ty in 0..geometry.count(rows.len()) {
                write_tiles(&mut ctx, &spec, level, ty, 0, &rows, &fraction_code)?;
            }
        }
        layer_meta.insert(spec.name.into(), metadata(&spec, spacing, last));
    }
    ensure(
        ctx.checks.shared_edge_mismatches == 0,
        format!(
            "tile edges differ from their neighbours: {} texels ({})",
            ctx.checks.shared_edge_mismatches,
            ctx.checks.examples.join("; ")
        ),
    )?;
    ctx.writer.finish()?;

    let levels: Vec<Value> = (0..=last)
        .map(|level| {
            let samples = w >> level;
            let count = geometry.count(samples);
            // Only the last tile of each row and column can be smaller.
            json!({"level": level, "spacing_m": f64::from(1u32 << level) * spacing,
                "tiles_per_side": count, "samples_per_side": samples,
                "tile_texels": geometry.per_tile(), "last_tile_texels": geometry.span(count - 1, samples).1})
        })
        .collect();
    let crs = input["crs"].as_str().unwrap_or_default().to_owned();
    let mut master_files = Vec::new();
    if masters {
        let georef = vec![
            "-a_srs".into(),
            crs.clone(),
            "-a_ullr".into(),
            origin[0].to_string(),
            origin[1].to_string(),
            (origin[0] + w as f64 * spacing).to_string(),
            (origin[1] - h as f64 * spacing).to_string(),
        ];
        geotiff(stage.path(), &mut elevation, "elevation", h, &georef)?;
        master_files.push("elevation.tif (Float32 m COG)".to_owned());
    }
    let result = json!({
        "schema_version": 2,
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
        "lod": "area mean pyramid: level k+1 texel i is the [1/4, 1/2, 1/4]² footprint mean of level k samples 2i-1..2i+1 (mode for classes); texel i of level k sits at level-0 sample i·2^k",
        "apron_texels": APRON,
        "encoding": {"format": "PNG grayscale 16-bit", "h_min_m": h_min, "h_max_m": h_max, "step_m": step,
            "decode": "height_m = h_min_m + code * step_m", "range_from": "ground and water surface",
            "max_quantization_error_m": max_error},
        "layers": layer_meta,
        "tile_path": "{layer}/L{level}/{prefix}_L{level}_{x}_{y}.png; prefix in layers.{layer}.prefix; tile texel (A, A) of a layer with apron A is level sample (x·intervals, y·intervals), aprons beyond the world edge repeat the edge texel",
        "levels": levels,
        "masters": master_files,
        "attribution": [&input["base"]["attribution"], &input["cover"]["attribution"]],
        "checksums": "checksums.txt: SHA-256 of every tile, `sha256sum -c checksums.txt`",
    });
    crate::acquisition::write_new(
        &stage.path().join("manifest.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    world_readable(stage.path())?;
    io_context("publish export", fs::rename(stage.path(), output))?;
    Ok(output.join("manifest.json"))
}

#[cfg(test)]
mod tests {
    use super::super::{testworld, view};
    use super::*;

    #[test]
    fn v2_tiles_share_edges_and_carry_aprons() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 512, 1.0, |x, y| {
            ((x + 2 * y) as f32 * 0.1, f32::NAN, 0)
        });
        testworld::game(&world, 1, |x, y| {
            [(x / 40 + 1) as u8, 30, (y % 256) as u8, 0, 0]
        });
        let out = dir.path().join("export");
        heightmap(&world, &out, 16, Layout::SharedEdge, false).unwrap();
        let m: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["schema_version"], 2);
        assert!(m["layers"]["biome"].is_object() && m["layers"].get("vegetation").is_none());
        // Shared edge: tile (0,0)'s last height column is tile (1,0)'s first.
        let (w, _, a) = view::read_png(&out.join("height/L0/height_L0_0_0.png")).unwrap();
        let (_, _, b) = view::read_png(&out.join("height/L0/height_L0_1_0.png")).unwrap();
        assert_eq!(w, 17);
        for r in 0..17 {
            assert_eq!(a[2 * (r * 17 + 16)..][..2], b[2 * (r * 17)..][..2]);
        }
        // Apron: biome L1 tiles are 17 + 8 wide; tile (1,0)'s apron repeats tile (0,0)'s columns.
        let (bw, _, p) = view::read_png(&out.join("biome/L1/biome_L1_0_0.png")).unwrap();
        let (_, _, q) = view::read_png(&out.join("biome/L1/biome_L1_1_0.png")).unwrap();
        assert_eq!(bw, 25);
        for r in 0..25 {
            for c in 0..8 {
                assert_eq!(q[r * 25 + c], p[r * 25 + 16 + c]);
            }
        }
        // No water anywhere: no water files.
        assert!(!out.join("water-coverage").exists());
    }

    #[test]
    fn coarse_heights_are_area_means() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, _| {
            (if x == 64 { 4.0 } else { 0.0 }, f32::NAN, 0)
        });
        let out = dir.path().join("export");
        heightmap(&world, &out, 16, Layout::SharedEdge, false).unwrap();
        let m: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        let (h_min, step) = (
            m["encoding"]["h_min_m"].as_f64().unwrap(),
            m["encoding"]["step_m"].as_f64().unwrap(),
        );
        // L1 sample 32 is column 0 of tile 2.
        let (w, _, t) = view::read_png(&out.join("height/L1/height_L1_2_3.png")).unwrap();
        for y in 0..w {
            let code = u16::from_be_bytes([t[2 * y * w], t[2 * y * w + 1]]);
            let height = h_min + f64::from(code) * step;
            assert!((height - 2.0).abs() <= step, "{height}");
        }
    }
}
