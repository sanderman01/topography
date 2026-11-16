//! Heightmap and game layer texture tiles with area-mean quadtree LOD levels
//! (master plan §5, §10; manifest v3), and partial re-exports that reuse unchanged layers.

use super::checks::Checks;
use super::description::{self, Frame};
use super::layers::{
    APRON, GAME_CLASSES, GAME_DENSITIES, HEIGHT, LayerSpec, MOISTURE, WATER_BODY, WATER_COVERAGE,
    WATER_STEP_M, WATER_SURFACE, WATER_UNWRAP_LIMIT_M, metadata,
};
use super::legends::legends;
use super::normals::{Baker, NormalOptions};
use super::pyramid::{ClassMean, Mean, Pyramid, Reduce, Shares, shares, top_two};
use super::tiles::{Geometry, RowWindow, TileWriter, cut};
use super::water::{
    Bodies, Kind, REACH, Terrain, TerrainRow, WaterStats, extended_surface, fill_diagonals, level0,
};
use crate::acquisition::{
    AcquisitionError, Result, digest, ensure, hash_file, io_context, raster, read_small,
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// Manifest schema version; also part of every input digest, so a format change never reuses
/// layers of an older export.
const SCHEMA_VERSION: u64 = 3;
/// The old 128 m moisture grid starts at the first level at least this coarse.
const MOISTURE_START_M: f64 = 16.0;

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

/// A partial export: rebuild `layers` (group names) and reuse every other group from `from`.
pub struct Partial {
    pub from: PathBuf,
    pub layers: Vec<String>,
}

/// Layer groups that rebuild together. Game layer groups are their layer names.
const TERRAIN: &str = "terrain";
const DESCRIPTION: &str = "description";
const NORMALS: &str = "normals";
const MOISTURE_GROUP: &str = "moisture";

/// Paths below the export root that belong to `group`.
fn group_paths(group: &str) -> Vec<&str> {
    match group {
        TERRAIN => vec![
            "height",
            "water-surface",
            "water-coverage",
            "water-body",
            "elevation.tif",
        ],
        DESCRIPTION => vec!["description.json", "description.md"],
        NORMALS => vec!["normal", "roughness"],
        other => vec![other],
    }
}

/// Manifest `layers` keys of `group`.
fn group_layers(group: &str) -> Vec<&str> {
    match group {
        TERRAIN => vec!["height", "water-surface", "water-coverage", "water-body"],
        DESCRIPTION => vec![],
        NORMALS => vec!["normal", "roughness"],
        other => vec![other],
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

fn ids(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
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

/// Window indices of the rows of tile row `ty` with `apron` (clamped); the window starts at
/// world row `first`.
fn tile_rows(ctx: &Ctx, level: u32, ty: usize, first: usize, apron: usize) -> Vec<usize> {
    let samples = ctx.world >> level;
    let (y0, th) = ctx.geometry.span(ty, samples);
    (0..th + 2 * apron)
        .map(|r| (y0 + r).saturating_sub(apron).min(samples - 1) - first)
        .collect()
}

/// Appends a texel's bytes (big-endian for 16 bits, one byte per channel otherwise).
type Code<'a, T> = &'a dyn Fn(T, &mut Vec<u8>);

fn encode<T: Copy>(values: &[T], code: Code<'_, T>) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for &v in values {
        code(v, &mut out);
    }
    out
}

/// Cuts, encodes and writes tile row `ty` of one level; `rows` start at world row `first`.
fn write_tiles<T: Copy>(
    ctx: &mut Ctx,
    spec: &LayerSpec,
    level: u32,
    ty: usize,
    first: usize,
    rows: &[&[T]],
    code: Code<'_, T>,
) -> Result<()> {
    let samples = ctx.world >> level;
    let picked = tile_rows(ctx, level, ty, first, spec.apron);
    let h = picked.len();
    let tile_rows: Vec<&[T]> = picked.iter().map(|&i| rows[i]).collect();
    for tx in 0..ctx.geometry.count(samples) {
        let values = cut(&tile_rows, ctx.geometry, samples, tx, spec.apron);
        let texels = encode(&values, code);
        let w = values.len() / h;
        ctx.writer
            .write(spec, level, tx, ty, w, h, &texels, &mut ctx.checks)?;
    }
    Ok(())
}

/// `water-surface` code: 2 cm steps from 0 m, wrapped every 65,536 codes.
fn water_code(s: f32) -> u16 {
    ((f64::from(s) / WATER_STEP_M).round() as i64).rem_euclid(65536) as u16
}

/// One layer's row windows per level, fed by its pyramid's sink.
struct Output<'a, T: Clone> {
    spec: LayerSpec,
    windows: Vec<RowWindow<Vec<T>>>,
    code: Code<'a, T>,
}

/// Height and the three water layers, cut from terrain row windows.
struct TerrainOutput<'a> {
    height: LayerSpec,
    /// Surface, coverage, body.
    water: [LayerSpec; 3],
    windows: Vec<RowWindow<TerrainRow>>,
    code: Code<'a, f32>,
    bodies: &'a Bodies,
}

impl<'a> TerrainOutput<'a> {
    fn new(
        height: LayerSpec,
        last: u32,
        ctx: &Ctx,
        code: Code<'a, f32>,
        bodies: &'a Bodies,
    ) -> Self {
        let water = [WATER_SURFACE, WATER_COVERAGE, WATER_BODY];
        // The surface extension looks REACH rows past each apron.
        let windows = (0..=last)
            .map(|l| RowWindow::new(ctx.geometry, ctx.world >> l, APRON + REACH))
            .collect();
        Self {
            height,
            water,
            windows,
            code,
            bodies,
        }
    }

    fn row(&mut self, ctx: &mut Ctx, level: u32, y: usize, row: &TerrainRow) -> Result<()> {
        if level > 0 {
            let below = (0..row.body.len())
                .filter(|&x| row.coverage[x] >= 0.5 && row.surface[x] < row.ground[x])
                .count() as u64;
            ctx.checks.coarse_wet_below_ground[level as usize - 1] += below;
        }
        let Some((ty, first, rows)) = self.windows[level as usize].push(y, row.clone()) else {
            return Ok(());
        };
        let ground: Vec<&[f32]> = rows.iter().map(|r| r.ground.as_slice()).collect();
        write_tiles(ctx, &self.height, level, ty, first, &ground, self.code)?;

        let refs: Vec<&TerrainRow> = rows.iter().collect();
        let mut extended = extended_surface(&refs, REACH);
        // An extension too far from the ground to unwrap (water below a cliff) is dropped.
        for ((surface, body), r) in extended.iter_mut().zip(&rows) {
            for x in 0..r.body.len() {
                let gap = f64::from((surface[x] - r.ground[x]).abs());
                if r.body[x] == 0 && body[x] > 0 && gap >= WATER_UNWRAP_LIMIT_M {
                    (surface[x], body[x]) = (f32::NAN, 0);
                }
            }
        }
        let picked = tile_rows(ctx, level, ty, first, APRON);
        let h = picked.len();
        let surface: Vec<&[f32]> = picked.iter().map(|&i| &extended[i].0[..]).collect();
        let body: Vec<&[u16]> = picked.iter().map(|&i| &extended[i].1[..]).collect();
        let coverage: Vec<&[f32]> = picked.iter().map(|&i| &rows[i].coverage[..]).collect();
        let ground: Vec<&[f32]> = picked.iter().map(|&i| &rows[i].ground[..]).collect();
        let (g, samples) = (ctx.geometry, ctx.world >> level);
        for tx in 0..g.count(samples) {
            let b = cut(&body, g, samples, tx, APRON);
            if !b.iter().any(|&id| id > 0) {
                continue;
            }
            let c = cut(&coverage, g, samples, tx, APRON);
            let s = cut(&surface, g, samples, tx, APRON);
            let gr = cut(&ground, g, samples, tx, APRON);
            // A share that rounds to 0 is written as 1, so wet ⇔ coverage > 0.
            let cb: Vec<u8> = c
                .iter()
                .map(|&v| ((v * 255.0).round().clamp(0.0, 255.0) as u8).max(u8::from(v > 0.0)))
                .collect();
            let w = cb.len() / h;
            // Coarse mixed texels (a canyon stream or the sea under a high mean ground) are
            // written within the unwrap limit of their ground, so every texel decodes to the
            // right wrap; they are still counted, and level_m stays authoritative.
            let limit = WATER_UNWRAP_LIMIT_M as f32;
            let codes: Vec<u16> = (0..s.len())
                .map(|i| match b[i] {
                    0 => 0,
                    _ if level > 0 => water_code(s[i].clamp(gr[i] - limit, gr[i] + limit)),
                    _ => water_code(s[i]),
                })
                .collect();
            let tile = WaterTile {
                level,
                tile: (tx, ty),
                w,
                surface: &s,
                coverage: &c,
                coverage_codes: &cb,
                body: &b,
                ground: &gr,
            };
            self.check_tile(ctx, &tile);
            let st: Vec<u8> = codes.iter().flat_map(|c| c.to_be_bytes()).collect();
            let bt: Vec<u8> = b.iter().flat_map(|c| c.to_be_bytes()).collect();
            let [ss, cs, bs] = &self.water;
            for (spec, t) in [(ss, st), (cs, cb), (bs, bt)] {
                ctx.writer
                    .write(spec, level, tx, ty, w, h, &t, &mut ctx.checks)?;
            }
        }
        Ok(())
    }

    /// Records lake and sea surface codes of a water tile (before the coarse clamp); checks
    /// over its interior that the surface can be unwrapped against the ground, that coverage
    /// > 0 has a body and, at level 0, that coverage is 255 exactly where the texel is wet.
    fn check_tile(&self, ctx: &mut Ctx, t: &WaterTile) {
        let mut last = (0u16, 0u16);
        for (&id, &s) in t.body.iter().zip(t.surface) {
            let code = water_code(s);
            if id > 0 && (id, code) != last {
                last = (id, code);
                if self.bodies.kind[usize::from(id)] != Kind::River {
                    ctx.checks.record_lake(t.level, id, code);
                }
            }
        }
        let (g, samples) = (ctx.geometry, ctx.world >> t.level);
        let (x0, tw) = g.span(t.tile.0, samples);
        let (y0, th) = g.span(t.tile.1, samples);
        let rows = t.body.len() / t.w;
        for r in APRON..(APRON + th).min(rows) {
            for k in APRON..APRON + tw {
                let i = r * t.w + k;
                let (x, y) = (x0 + k - APRON, y0 + r - APRON);
                let level = t.level;
                // Extensions this far were dropped; coarse mixed shore texels (sea under a
                // cliff) are a figure, level 0 fails.
                let gap = f64::from(t.surface[i] - t.ground[i]).abs();
                if t.body[i] > 0 && (gap.is_nan() || gap >= WATER_UNWRAP_LIMIT_M) {
                    let river = self.bodies.kind[usize::from(t.body[i])] == Kind::River;
                    if level > 0 {
                        let k = level as usize - 1;
                        ctx.checks.coarse_far_from_ground[k][usize::from(river)] += 1;
                    } else {
                        ctx.checks.surface_far_from_ground += 1;
                        ctx.checks.example("surface_far_from_ground", || {
                            format!(
                                "L{level} at ({x}, {y}): surface {} m, ground {} m",
                                t.surface[i], t.ground[i]
                            )
                        });
                    }
                }
                let expected = if t.level == 0 {
                    if t.coverage[i] > 0.0 { 255 } else { 0 }
                } else {
                    t.coverage_codes[i]
                };
                let orphan = t.coverage_codes[i] > 0 && t.body[i] == 0;
                if orphan || t.coverage_codes[i] != expected {
                    ctx.checks.coverage_mismatch += 1;
                    ctx.checks.example("coverage_mismatch", || {
                        format!(
                            "L{level} at ({x}, {y}): coverage {}, export id {}",
                            t.coverage_codes[i], t.body[i]
                        )
                    });
                }
            }
        }
    }
}

/// One water tile with its apron, row-major `w` wide.
struct WaterTile<'a> {
    level: u32,
    tile: (usize, usize),
    w: usize,
    surface: &'a [f32],
    coverage: &'a [f32],
    coverage_codes: &'a [u8],
    body: &'a [u16],
    ground: &'a [f32],
}

impl<'a, T: Copy> Output<'a, T> {
    fn new(spec: LayerSpec, last: u32, ctx: &Ctx, code: Code<'a, T>) -> Self {
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
                let rows: Vec<&[T]> = rows.iter().map(Vec::as_slice).collect();
                write_tiles(ctx, &self.spec, level, ty, first, &rows, self.code)
            }
            None => Ok(()),
        }
    }
}

/// The game layer groups of `manifest["game"]`: level and each group's world files.
struct GameGroups {
    level: u32,
    groups: Vec<(String, Vec<String>)>,
}

fn game_groups(manifest: &Value, w: usize) -> Result<Option<GameGroups>> {
    let Some(game) = manifest.get("game") else {
        return Ok(None);
    };
    let level = game["level"].as_u64().unwrap_or(u64::MAX);
    ensure(level <= 1, "game layers must be at level 0 or 1")?;
    let level = level as u32;
    let m = w >> level;
    let size: [usize; 2] = serde_json::from_value(game["size"].clone())?;
    ensure(size == [m, m], "game layer size does not match the world")?;
    let names: BTreeSet<String> = serde_json::from_value(game["layers"].clone())?;
    let file = |n: &str| format!("game/{n}.u8");
    let mut groups = Vec::new();
    for spec in &GAME_CLASSES {
        ensure(
            !names.contains(spec.name),
            format!(
                "game layer {} is the dithered v1 class layer; rerun game_layers to write {0}-a, {0}-b and {0}-weight",
                spec.name
            ),
        )?;
        let parts: Vec<String> = ["a", "b", "weight"]
            .iter()
            .map(|p| format!("{}-{p}", spec.name))
            .collect();
        let found = parts.iter().filter(|p| names.contains(*p)).count();
        ensure(
            found == 0 || found == 3,
            format!("game layers need all of {}", parts.join(", ")),
        )?;
        if found == 3 {
            groups.push((spec.name.into(), parts.iter().map(|p| file(p)).collect()));
        }
    }
    for name in &names {
        let class = GAME_CLASSES.iter().any(|s| {
            name.strip_prefix(s.name)
                .is_some_and(|r| r.starts_with('-'))
        });
        if class {
            continue;
        }
        let known = GAME_DENSITIES.iter().any(|s| s.name == name) || name == MOISTURE.name;
        ensure(known, format!("unknown game layer {name}"))?;
        groups.push((name.clone(), vec![file(name)]));
    }
    Ok(Some(GameGroups { level, groups }))
}

/// The old moisture source: the world's 128 m grid, used when the game layers have none.
fn old_moisture(manifest: &Value) -> Option<&Value> {
    manifest.get("moisture")
}

/// The first moisture level, built whole by bilinear sampling of the world's moisture grid
/// at texel positions.
fn moisture(
    world: &Path,
    entry: &Value,
    layers: &Value,
    origin: [f64; 2],
    (w, spacing): (usize, f64),
    last: u32,
) -> Result<(u32, Vec<Vec<f32>>)> {
    let [mw, mh]: [usize; 2] = serde_json::from_value(entry["size"].clone())?;
    let [nx, ny]: [f64; 2] = serde_json::from_value(entry["nw_m"].clone())?;
    let ms = entry["spacing_m"].as_f64().unwrap_or(0.0);
    let file = entry["file"].as_str().unwrap_or_default();
    ensure(mw > 0 && mh > 0 && ms > 0.0, "invalid world moisture entry")?;
    let mut layer = Layer::open(world, file, layers, (mw, mh), 4)?;
    let mut buf = Vec::new();
    layer.rows(0, mh, &mut buf)?;
    let grid: Vec<f32> = floats(&buf).collect();
    let level = moisture_level(spacing, last);
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
    Ok((level, rows))
}

fn moisture_level(spacing: f64, last: u32) -> u32 {
    (0..last)
        .find(|&k| spacing * f64::from(1u32 << k) >= MOISTURE_START_M)
        .unwrap_or(last)
}

/// Bytes available to this user on the file system of `dir` (POSIX `df`).
fn free_bytes(dir: &Path) -> Result<u64> {
    let out = io_context(
        "run df",
        std::process::Command::new("df")
            .args(["-Pk"])
            .arg(dir)
            .output(),
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let kb = text
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|v| v.parse::<u64>().ok());
    match kb {
        Some(kb) if out.status.success() => Ok(kb * 1024),
        _ => Err(AcquisitionError::Invalid(format!(
            "cannot read free space of {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
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

/// Hardlinks a file or directory tree (copying where links fail, e.g. across file systems).
fn link_tree(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        io_context("create reused layer", fs::create_dir_all(dst))?;
        for entry in io_context("list reused layer", fs::read_dir(src))? {
            let entry = io_context("list reused layer", entry)?;
            link_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if fs::hard_link(src, dst).is_err() {
            io_context(
                format!("copy {}", src.display()),
                fs::copy(src, dst).map(|_| ()),
            )?;
        }
        Ok(())
    }
}

/// Input digest of every layer group the world can export: the world files' SHA-256 from its
/// manifest, plus the settings that shape the group's tiles.
fn input_digests(
    world: &Path,
    manifest: &Value,
    game: Option<&GameGroups>,
    normals: NormalOptions,
    (intervals, layout): (usize, Layout),
) -> BTreeMap<String, Value> {
    let files = |names: &[&str]| -> Value {
        names
            .iter()
            .map(|n| ((*n).to_owned(), manifest["layers"][*n].clone()))
            .collect::<Map<String, Value>>()
            .into()
    };
    let common =
        json!({"schema_version": SCHEMA_VERSION, "intervals": intervals, "layout": layout.name()});
    let water = [
        "elevation.f32",
        "water-surface.f32",
        "water-body.u32",
        "bodies.json",
    ];
    let mut out = BTreeMap::new();
    out.insert(
        TERRAIN.to_owned(),
        json!({"files": files(&water), "export": common}),
    );
    out.insert(
        DESCRIPTION.to_owned(),
        json!({"files": files(&water), "export": common,
            "other": description::inputs(world, manifest)}),
    );
    if normals.enabled {
        out.insert(
            NORMALS.to_owned(),
            json!({"files": files(&["elevation.f32"]), "export": common,
                "options": {"start_level": normals.start_level, "apron": normals.apron,
                    "roughness": normals.roughness}}),
        );
    }
    let mut game_moisture = false;
    if let Some(g) = game {
        for (group, names) in &g.groups {
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            out.insert(
                group.clone(),
                json!({"files": files(&names), "export": common, "level": g.level}),
            );
            game_moisture |= group == MOISTURE_GROUP;
        }
    }
    if let Some(entry) = old_moisture(manifest).filter(|_| !game_moisture) {
        let file = entry["file"].as_str().unwrap_or_default();
        out.insert(
            MOISTURE_GROUP.to_owned(),
            json!({"files": files(&[file]), "export": common, "start_m": MOISTURE_START_M}),
        );
    }
    for (group, entry) in &mut out {
        entry["exporter_version"] = json!(group_version(group));
    }
    out
}

/// Report checks and figures that a group's pass measures; shared-edge checks are per
/// rebuilt layer and stay this export's.
fn group_report_keys(group: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match group {
        TERRAIN => (
            &[
                "wet_not_above_ground",
                "wet_without_surface",
                "surface_without_body",
                "lake_levels_off",
                "lake_levels",
                "coverage_mismatch",
                "surface_far_from_ground",
            ],
            &[
                "coarse_wet_below_ground",
                "coarse_surface_far_from_ground",
                "lake_levels_checked",
                "diagonal_fills",
                "diagonal_fills_skipped",
            ],
        ),
        NORMALS => (&["bc_gross_errors"], &["bc_error_budget"]),
        _ => (&[], &[]),
    }
}

/// Version of the code that writes a layer group: bump it whenever that group's output
/// changes, so partial exports never reuse tiles of older code.
fn group_version(group: &str) -> u32 {
    match group {
        // 2: coarse flat-body texels clamped to the unwrap limit.
        TERRAIN => 2,
        DESCRIPTION => 1,
        NORMALS => 1,
        MOISTURE_GROUP => 1,
        // Class layers (share mean) and density-like game layers.
        _ if GAME_CLASSES.iter().any(|s| s.name == group) => 1,
        _ => 1,
    }
}

/// The previous export of a partial run, checked against this one.
struct Previous {
    dir: PathBuf,
    manifest: Value,
    /// Location-free identity for the new manifest: directory name and manifest SHA-256.
    id: Value,
    /// Groups reused from it.
    reused: Vec<String>,
}

fn previous(
    partial: &Partial,
    digests: &BTreeMap<String, Value>,
    current: &Value,
) -> Result<(Previous, BTreeSet<String>)> {
    let dir = partial.from.canonicalize().map_err(|e| {
        AcquisitionError::Invalid(format!("previous export {}: {e}", partial.from.display()))
    })?;
    let manifest: Value = serde_json::from_slice(&read_small(&dir.join("manifest.json"))?)?;
    ensure(
        manifest["kind"] == "heightmap_export" && manifest["schema_version"] == SCHEMA_VERSION,
        format!(
            "{} is not a schema {SCHEMA_VERSION} heightmap export: run a full export",
            dir.display()
        ),
    )?;
    let prev = manifest["input_digests"].as_object().ok_or_else(|| {
        AcquisitionError::Invalid(format!(
            "{} records no input digests: run a full export",
            dir.display()
        ))
    })?;
    for key in [
        "intervals",
        "layout",
        "size",
        "spacing_m",
        "origin_nw_corner",
    ] {
        ensure(
            manifest[key] == current[key],
            format!("previous export differs in {key}: run a full export"),
        )?;
    }
    let mut build = BTreeSet::new();
    for name in &partial.layers {
        ensure(
            digests.contains_key(name),
            format!(
                "unknown or absent layer group {name}; this world has {}",
                digests.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        )?;
        build.insert(name.clone());
    }
    ensure(!build.is_empty(), "--layers names no layer group")?;
    if build.contains(TERRAIN) {
        build.insert(DESCRIPTION.to_owned());
    }
    let mut reused = Vec::new();
    for (group, digest) in digests.iter().filter(|(g, _)| !build.contains(*g)) {
        match prev.get(group) {
            None => {
                return Err(AcquisitionError::Invalid(format!(
                    "previous export has no {group} layers: add {group} to --layers"
                )));
            }
            Some(old) if old != digest => {
                return Err(AcquisitionError::Invalid(format!(
                    "layer group {group} of the previous export was made from other inputs: add {group} to --layers"
                )));
            }
            Some(_) => reused.push(group.clone()),
        }
    }
    Ok((
        Previous {
            id: json!({
                "name": dir.file_name().map(|n| n.to_string_lossy().into_owned()),
                "manifest_sha256": digest(&read_small(&dir.join("manifest.json"))?),
            }),
            dir,
            manifest,
            reused,
        },
        build,
    ))
}

/// Level-0 rows with diagonal river corners filled, one row behind the input.
struct Filler<'a> {
    bodies: &'a Bodies,
    pending: Option<(usize, TerrainRow)>,
    /// Filled and skipped corners.
    fills: [u64; 2],
}

impl<'a> Filler<'a> {
    fn new(bodies: &'a Bodies) -> Self {
        Self {
            bodies,
            pending: None,
            fills: [0; 2],
        }
    }

    fn push(&mut self, y: usize, mut row: TerrainRow) -> Option<(usize, TerrainRow)> {
        let out = self.pending.take().map(|(py, mut prev)| {
            let [f, s] = fill_diagonals(&mut prev, &mut row, self.bodies);
            self.fills[0] += f;
            self.fills[1] += s;
            (py, prev)
        });
        self.pending = Some((y, row));
        out
    }

    fn finish(&mut self) -> Option<(usize, TerrainRow)> {
        self.pending.take()
    }
}

/// Export a generated world as 16-bit height tiles, game layer and moisture tiles with
/// aprons (manifest v3), and baked normal and roughness KTX2 tiles per `normals`; with
/// `masters`, also a Float32 elevation COG.
pub fn heightmap(
    world: &Path,
    output: &Path,
    intervals: usize,
    layout: Layout,
    masters: bool,
    normals: NormalOptions,
) -> Result<PathBuf> {
    export(world, output, intervals, layout, masters, normals, None)
}

/// Like `heightmap`, rebuilding only `partial.layers` and hardlinking every other layer group
/// from `partial.from`, whose recorded input digests must match the world's.
pub fn heightmap_from(
    world: &Path,
    output: &Path,
    intervals: usize,
    layout: Layout,
    masters: bool,
    normals: NormalOptions,
    partial: &Partial,
) -> Result<PathBuf> {
    export(
        world,
        output,
        intervals,
        layout,
        masters,
        normals,
        Some(partial),
    )
}

fn export(
    world: &Path,
    output: &Path,
    intervals: usize,
    layout: Layout,
    masters: bool,
    normals: NormalOptions,
    partial: Option<&Partial>,
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
        "world has no water layers; regenerate it with hydrology v3",
    )?;
    let mut last = 0u32;
    while (w >> last) > intervals {
        last += 1;
    }
    normals.validate(intervals, last)?;
    let game = game_groups(&manifest, w)?;
    if let Some(game) = &game {
        ensure(
            game.level <= last,
            "game layers are coarser than the export's last level",
        )?;
    }
    let digests = input_digests(
        world,
        &manifest,
        game.as_ref(),
        normals,
        (intervals, layout),
    );
    let frame_json = json!({"intervals": intervals, "layout": layout.name(), "size": [w, h],
        "spacing_m": spacing, "origin_nw_corner": origin});
    let (prev, build) = match partial {
        Some(p) => {
            let (prev, build) = previous(p, &digests, &frame_json)?;
            (Some(prev), build)
        }
        None => (None, digests.keys().cloned().collect()),
    };
    let built = |g: &str| build.contains(g);
    ensure(
        !masters || built(TERRAIN),
        "--masters needs the terrain layers rebuilt",
    )?;
    if !digests.contains_key(MOISTURE_GROUP) {
        eprintln!("world has no moisture; moisture tiles skipped");
    }
    let bodies = Bodies::load(world)?;
    let need_terrain_pass = built(TERRAIN) || built(DESCRIPTION);
    let need_elevation = need_terrain_pass || built(NORMALS);
    let mut elevation = need_elevation
        .then(|| Layer::open(world, "elevation.f32", layers, (w, h), 4))
        .transpose()?;
    let mut water_layers = need_terrain_pass
        .then(|| -> Result<_> {
            Ok((
                Layer::open(world, "water-surface.f32", layers, (w, h), 4)?,
                Layer::open(world, "water-body.u32", layers, (w, h), 4)?,
            ))
        })
        .transpose()?;

    // Range from ground, finite water surfaces and the levels of lakes and seas with cells:
    // terrain continues under water. Level-0 water problems fail here, before the long pass.
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut stats = WaterStats::new(&bodies);
    let (mut buf, mut surface_buf, mut body_buf) = (Vec::new(), Vec::new(), Vec::new());
    if let (Some(elevation), Some((surface, body))) = (&mut elevation, &mut water_layers) {
        let mut present = vec![false; bodies.len()];
        let mut early = Checks::new(0);
        let mut filler = Filler::new(&bodies);
        for y0 in (0..h).step_by(CHUNK_ROWS) {
            let count = CHUNK_ROWS.min(h - y0);
            elevation.rows(y0, count, &mut buf)?;
            surface.rows(y0, count, &mut surface_buf)?;
            body.rows(y0, count, &mut body_buf)?;
            for e in floats(&buf) {
                ensure(e.is_finite(), "non-finite elevation")?;
                lo = lo.min(e);
                hi = hi.max(e);
            }
            for s in floats(&surface_buf).filter(|s| s.is_finite()) {
                lo = lo.min(s);
                hi = hi.max(s);
            }
            let ids = ids(&body_buf);
            let mut previous = 0;
            for &b in &ids {
                if b != 0 && b != previous {
                    previous = b;
                    if let Some(id) = bodies.export_id(b) {
                        present[usize::from(id)] = true;
                    }
                }
            }
            for r in 0..count {
                let line = 4 * w * r..4 * w * (r + 1);
                let ground: Vec<f32> = floats(&buf[line.clone()]).collect();
                let water: Vec<f32> = floats(&surface_buf[line]).collect();
                let ids = &ids[w * r..w * (r + 1)];
                let row = level0(&ground, &water, ids, &bodies, &mut early, y0 + r)?;
                if let Some((y, row)) = filler.push(y0 + r, row) {
                    stats.add(&row, y);
                }
            }
        }
        if let Some((y, row)) = filler.finish() {
            stats.add(&row, y);
        }
        ensure(
            early.passed(),
            format!("world checks failed: {}", early.failures()),
        )?;
        // Lakes and seas are written at their level, which must not be clamped.
        for (_, &level) in present
            .iter()
            .zip(&bodies.level_m)
            .filter(|(p, l)| **p && l.is_finite())
        {
            lo = lo.min(level);
            hi = hi.max(level);
        }
        ensure(hi > lo, "world needs a positive height range")?;
    }
    // 65,534 steps over the range and h_min on the step lattice, so 0 m is a code.
    let step = f64::from(hi - lo) / 65534.0;
    let h_min = -(-f64::from(lo) / step).ceil() * step;
    let h_max = h_min + 65535.0 * step;
    let height_code = |e: f32| (((f64::from(e) - h_min) / step).round()).clamp(0.0, 65535.0) as u16;
    let height_bytes = |e: f32, out: &mut Vec<u8>| out.extend(height_code(e).to_be_bytes());
    let class_bytes = |v: [u8; 3], out: &mut Vec<u8>| out.extend(v);
    let density_bytes = |v: f32, out: &mut Vec<u8>| out.push(v.round().clamp(0.0, 255.0) as u8);
    let fraction_bytes =
        |v: f32, out: &mut Vec<u8>| out.push((v * 255.0).round().clamp(0.0, 255.0) as u8);
    let mut max_error = 0f64;

    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create export parent", fs::create_dir_all(parent))?;
    // Measured exports take about 1.3 B per level-0 sample; masters add a raw and a COG copy.
    let need = if built(TERRAIN) {
        (w * h) as u64 * if masters { 10 } else { 2 }
    } else {
        (w * h) as u64 / 2
    };
    let free = free_bytes(parent)?;
    ensure(
        free >= need,
        format!(
            "export needs about {:.1} GB free beside the output, {:.1} GB available",
            need as f64 / 1e9,
            free as f64 / 1e9
        ),
    )?;
    // A killed export leaves this stage behind; remove it by hand.
    let name = output.file_name().unwrap_or_default().to_string_lossy();
    let stage = io_context(
        "stage export",
        tempfile::Builder::new()
            .prefix(&format!(".{name}.stage-"))
            .tempdir_in(parent),
    )?;
    let geometry = Geometry {
        intervals,
        shared: layout == Layout::SharedEdge,
    };
    let mut ctx = Ctx {
        writer: TileWriter::new(stage.path(), geometry.shared),
        checks: Checks::new(last),
        geometry,
        world: w,
    };

    let height_spec = LayerSpec {
        scale: step,
        offset: h_min,
        ..HEIGHT
    };
    let mut terrain = built(TERRAIN).then(|| {
        (
            Pyramid::new(Terrain { bodies: &bodies }, 0, last),
            TerrainOutput::new(height_spec, last, &ctx, &height_bytes, &bodies),
        )
    });
    let mut filler = Filler::new(&bodies);

    // Game layers being rebuilt.
    let game_level = game.as_ref().map_or(0, |g| g.level);
    let gw = w >> game_level;
    let mut classes = Vec::new();
    let mut densities = Vec::new();
    for (group, files) in game
        .iter()
        .flat_map(|g| &g.groups)
        .filter(|(g, _)| built(g))
    {
        let open = |f: &String| Layer::open(world, f, layers, (gw, gw), 1);
        if let Some(spec) = GAME_CLASSES.iter().find(|s| s.name == group) {
            let spec = LayerSpec {
                finest_level: game_level,
                ..*spec
            };
            let files = [open(&files[0])?, open(&files[1])?, open(&files[2])?];
            classes.push((
                Pyramid::new(ClassMean, game_level, last),
                Output::new(spec, last, &ctx, &class_bytes),
                files,
                [0u64; 256],
            ));
        } else {
            let spec = GAME_DENSITIES
                .iter()
                .chain(std::iter::once(&MOISTURE))
                .find(|s| s.name == group)
                .expect("game groups are known layers");
            let spec = LayerSpec {
                finest_level: game_level,
                ..*spec
            };
            densities.push((
                Pyramid::new(Mean, game_level, last),
                Output::new(spec, last, &ctx, &density_bytes),
                open(&files[0])?,
            ));
        }
    }

    let mut baker = (normals.enabled && built(NORMALS))
        .then(|| Baker::new(normals, intervals, w, spacing, last));
    let need_rows =
        terrain.is_some() || baker.is_some() || !classes.is_empty() || !densities.is_empty();
    let mut game_bufs = [Vec::new(), Vec::new(), Vec::new()];
    for y0 in (0..h).step_by(CHUNK_ROWS).filter(|_| need_rows) {
        let count = CHUNK_ROWS.min(h - y0);
        if let Some(e) = &mut elevation {
            e.rows(y0, count, &mut buf)?;
        }
        if let (Some(_), Some((surface, body))) = (&terrain, &mut water_layers) {
            surface.rows(y0, count, &mut surface_buf)?;
            body.rows(y0, count, &mut body_buf)?;
        }
        for r in 0..count {
            let y = y0 + r;
            let line = 4 * w * r..4 * w * (r + 1);
            let ground: Vec<f32> = if elevation.is_some() {
                floats(&buf[line.clone()]).collect()
            } else {
                Vec::new()
            };
            if let Some((p, out)) = &mut terrain {
                let water: Vec<f32> = floats(&surface_buf[line.clone()]).collect();
                let ids = ids(&body_buf[line]);
                for &e in &ground {
                    let decoded = h_min + f64::from(height_code(e)) * step;
                    max_error = max_error.max((decoded - f64::from(e)).abs());
                }
                let row = level0(&ground, &water, &ids, &bodies, &mut ctx.checks, y)?;
                if let Some((_, row)) = filler.push(y, row) {
                    p.push(row, &mut |l, y, row| out.row(&mut ctx, l, y, row))?;
                }
            }
            if let Some(b) = &mut baker {
                b.push(&ground, &mut ctx.writer, &mut ctx.checks)?;
            }
            // A level-1 game row is complete once its level-0 rows 2i-1..2i+1 are.
            let game_y = match game_level {
                0 => Some(y),
                _ => (y % 2 == 1).then_some(y / 2),
            };
            let Some(gy) = game_y else { continue };
            for (p, out, files, counts) in &mut classes {
                for (file, buf) in files.iter_mut().zip(&mut game_bufs) {
                    file.rows(gy, 1, buf)?;
                }
                let [a, b, weight] = &game_bufs;
                let row: Vec<Shares> = (0..gw)
                    .map(|x| {
                        counts[usize::from(a[x])] += 1;
                        if b[x] != a[x] && weight[x] < 255 {
                            counts[usize::from(b[x])] += 1;
                        }
                        shares(a[x], b[x], weight[x])
                    })
                    .collect();
                p.push(row, &mut |l, y, row| {
                    let texels: Vec<[u8; 3]> = row.iter().map(top_two).collect();
                    out.row(&mut ctx, l, y, &texels)
                })?;
            }
            for (p, out, layer) in &mut densities {
                layer.rows(gy, 1, &mut game_bufs[0])?;
                let row = game_bufs[0].iter().map(|&v| f32::from(v)).collect();
                p.push(row, &mut |l, y, row| out.row(&mut ctx, l, y, row))?;
            }
        }
    }
    if let Some((p, out)) = &mut terrain {
        if let Some((_, row)) = filler.finish() {
            p.push(row, &mut |l, y, row| out.row(&mut ctx, l, y, row))?;
        }
        [ctx.checks.diagonal_fills, ctx.checks.diagonal_fills_skipped] = filler.fills;
    }

    let mut layer_meta = Map::new();
    if let Some(b) = baker {
        layer_meta.extend(b.finish(&mut ctx.checks, spacing));
    }
    if terrain.is_some() {
        for spec in [height_spec, WATER_SURFACE, WATER_COVERAGE, WATER_BODY] {
            layer_meta.insert(spec.name.into(), metadata(&spec, spacing, last));
        }
    }
    let mut class_counts = [[0u64; 256]; 2];
    for (_, out, _, counts) in &classes {
        let i = usize::from(out.spec.name != GAME_CLASSES[0].name);
        class_counts[i] = *counts;
        layer_meta.insert(out.spec.name.into(), metadata(&out.spec, spacing, last));
    }
    let mut legends = legends(&class_counts[0], &class_counts[1])?;
    for (_, out, _) in &densities {
        layer_meta.insert(out.spec.name.into(), metadata(&out.spec, spacing, last));
    }
    let game_moisture = game
        .as_ref()
        .is_some_and(|g| g.groups.iter().any(|(n, _)| n == MOISTURE_GROUP));
    if let Some(entry) = old_moisture(&manifest).filter(|_| built(MOISTURE_GROUP) && !game_moisture)
    {
        let (first, mut rows) = moisture(world, entry, layers, origin, (w, spacing), last)?;
        let spec = LayerSpec {
            finest_level: first,
            filter: "area mean; first level bilinear from the world's 128 m grid",
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
            let refs: Vec<&[f32]> = rows.iter().map(Vec::as_slice).collect();
            for ty in 0..geometry.count(rows.len()) {
                write_tiles(&mut ctx, &spec, level, ty, 0, &refs, &fraction_bytes)?;
            }
        }
        layer_meta.insert(spec.name.into(), metadata(&spec, spacing, last));
    }
    // A failed check drops the stage unpublished.
    ensure(
        ctx.checks.passed(),
        format!("export checks failed: {}", ctx.checks.failures()),
    )?;
    let mut validation = ctx.checks.report();

    // Reused groups: hardlinked files, their manifest entries and checksum lines.
    let mut encoding = json!({"format": "PNG grayscale 16-bit", "h_min_m": h_min, "h_max_m": h_max,
        "step_m": step, "decode": "height_m = h_min_m + code * step_m",
        "range_from": "ground, water surface and lake and sea levels; h_min_m is a multiple of step_m, so 0 m is a code",
        "max_quantization_error_m": max_error});
    let mut description_entry = Value::Null;
    let mut master_files = Vec::new();
    let mut reused_layers = Map::new();
    if let Some(prev) = &prev {
        let mut dirs = BTreeSet::new();
        for group in &prev.reused {
            for path in group_paths(group) {
                let src = prev.dir.join(path);
                if src.exists() {
                    link_tree(&src, &stage.path().join(path))?;
                    dirs.insert(path.to_owned());
                }
            }
            for key in group_layers(group) {
                if let Some(m) = prev.manifest["layers"].get(key) {
                    layer_meta.insert(key.into(), m.clone());
                }
            }
            if GAME_CLASSES.iter().any(|s| s.name == group) {
                legends[group.as_str()] = prev.manifest["legends"][group.as_str()].clone();
            }
            reused_layers.insert(group.clone(), prev.id.clone());
        }
        let sums = io_context(
            "read previous checksums",
            fs::read_to_string(prev.dir.join("checksums.txt")),
        )?;
        let kept: String = sums
            .lines()
            .filter(|l| {
                l.split_once("  ")
                    .and_then(|(_, p)| p.split('/').next())
                    .is_some_and(|d| dirs.contains(d))
            })
            .map(|l| format!("{l}\n"))
            .collect();
        ctx.writer.add_checksums(&kept);
        if prev.reused.iter().any(|g| g == TERRAIN) {
            encoding = prev.manifest["encoding"].clone();
            master_files = serde_json::from_value(prev.manifest["masters"].clone())?;
        }
        if prev.reused.iter().any(|g| g == DESCRIPTION) {
            description_entry = prev.manifest["description"].clone();
        }
        let prev_passed = prev.manifest["validation"]["passed"] == true;
        // Checks and figures of reused groups come from the export that measured them.
        let old = &prev.manifest["validation"];
        let mut copied = Vec::new();
        for group in &prev.reused {
            let (checks, figures) = group_report_keys(group);
            for (section, keys) in [("checks", checks), ("figures", figures)] {
                for &key in keys {
                    if let Some(v) = old[section].get(key) {
                        validation[section][key] = v.clone();
                        copied.push(format!("{section}.{key}"));
                    }
                }
            }
        }
        validation["reused_from"] = json!({"export": prev.id,
            "groups": prev.reused, "passed": prev_passed, "copied": copied});
        validation["passed"] = json!(ctx.checks.passed() && prev_passed);
    }
    crate::acquisition::write_new(
        &stage.path().join("report.json"),
        &serde_json::to_vec_pretty(&validation)?,
    )?;
    ctx.writer.finish()?;
    if built(DESCRIPTION) {
        let frame = Frame {
            origin,
            spacing_m: spacing,
            size: w,
        };
        let sha = description::write(stage.path(), world, &manifest, &frame, &bodies, &stats)?;
        description_entry =
            json!({"file": "description.json", "sha256": sha, "schema": "world-description-v1"});
    }

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
        let elevation = elevation.as_mut().expect("terrain opens elevation");
        geotiff(stage.path(), elevation, "elevation", h, &georef)?;
        master_files.push("elevation.tif (Float32 m COG)".to_owned());
    }
    let mut result = json!({
        "schema_version": SCHEMA_VERSION,
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
        "lod": "area mean pyramid: level k+1 texel i is the [1/4, 1/2, 1/4]² footprint mean of level k samples 2i-1..2i+1 (class shares for the class layers, top two re-picked per level); texel i of level k sits at level-0 sample i·2^k",
        "start_levels": "each layer has tiles from layers.{layer}.levels[0]: the game layers (classes, densities, temperature, moisture) from the game level, the world's 128 m moisture from the first level of at least 16 m, normal and roughness from --normals-level; for a finer level read the ancestor tile at levels[0]",
        "apron_texels": APRON,
        "encoding": encoding,
        "layers": layer_meta,
        "legends": legends,
        "water_files": "water-surface, water-coverage and water-body tiles exist only where a texel of the tile or its apron has water-body > 0 (water or the extension past the shore); a missing file means coverage 0, body 0 and no surface",
        "tile_path": "{layer}/L{level}/{prefix}_L{level}_{x}_{y}.{ext}; prefix and file_extension (default png) in layers.{layer}; tile texel (A, A) of a layer with apron A is level sample (x·intervals, y·intervals), or cell (x·intervals, y·intervals) for the cell-centred normal and roughness layers; aprons beyond the world edge repeat the edge texel",
        "levels": levels,
        "masters": master_files,
        "validation": validation,
        "description": description_entry,
        "input_digests": digests,
        "attribution": [&input["base"]["attribution"], &input["cover"]["attribution"]],
        "checksums": "checksums.txt: SHA-256 of every tile, `sha256sum -c checksums.txt`",
    });
    if prev.is_some() {
        result["reused_layers"] = reused_layers.into();
    }
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
    fn v3_tiles_share_edges_and_carry_aprons() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 512, 1.0, |x, y| {
            ((x + 2 * y) as f32 * 0.1, f32::NAN, 0)
        });
        testworld::game(&world, 1, |x, y| testworld::Game {
            biome: [(x / 40 + 1) as u8, (x / 40 + 2) as u8, 200],
            land_cover: [30, 30, 255],
            densities: [(y % 256) as u8, 0, 0, 160, 128],
        });
        let out = dir.path().join("export");
        heightmap(
            &world,
            &out,
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
        )
        .unwrap();
        let m: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["schema_version"], 3);
        assert!(m["layers"]["biome"].is_object() && m["layers"].get("vegetation").is_none());
        assert_eq!(m["layers"]["biome"]["format"], "PNG RGB 8-bit");
        assert_eq!(m["layers"]["temperature"]["offset"], -30.0);
        assert_eq!(m["layers"]["moisture"]["levels"][0], 1);
        assert!(m["input_digests"]["biome"]["files"]["game/biome-a.u8"].is_string());
        let present = |legend: &str, id: u64| {
            m["legends"][legend]["classes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap()["present"]
                .as_bool()
        };
        assert_eq!(
            (present("land-cover", 30), present("land-cover", 10)),
            (Some(true), Some(false))
        );
        assert_eq!(
            (present("biome", 1), present("biome", 19)),
            (Some(true), Some(false))
        );
        // Shared edge: tile (0,0)'s last height column is tile (1,0)'s first.
        let (w, _, a) = view::read_png(&out.join("height/L0/height_L0_0_0.png")).unwrap();
        let (_, _, b) = view::read_png(&out.join("height/L0/height_L0_1_0.png")).unwrap();
        assert_eq!(w, 17);
        for r in 0..17 {
            assert_eq!(a[2 * (r * 17 + 16)..][..2], b[2 * (r * 17)..][..2]);
        }
        // Apron: biome L1 tiles are 17 + 8 wide RGB; tile (1,0)'s apron repeats tile (0,0)'s.
        let (bw, _, p) = view::read_png(&out.join("biome/L1/biome_L1_0_0.png")).unwrap();
        let (_, _, q) = view::read_png(&out.join("biome/L1/biome_L1_1_0.png")).unwrap();
        assert_eq!((bw, p.len()), (25, 25 * 25 * 3));
        for r in 0..25 {
            for c in 0..8 {
                assert_eq!(q[3 * (r * 25 + c)..][..3], p[3 * (r * 25 + 16 + c)..][..3]);
            }
        }
        // Game biome at L1 x = 0 (game texel 0): classes 1 and 2, weight 200.
        assert_eq!(p[3 * (4 * 25 + 4)..][..3], [1, 2, 200]);
        // No water anywhere: no water files.
        assert!(!out.join("water-coverage").exists());
    }

    #[test]
    fn dithered_class_layers_fail_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 64, 1.0, |x, _| (x as f32, f32::NAN, 0));
        let path = world.join("manifest.json");
        let mut m: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        m["game"] = json!({"level": 1, "size": [32, 32], "layers": ["biome"]});
        fs::write(&path, m.to_string()).unwrap();
        let err = heightmap(
            &world,
            &dir.path().join("x"),
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("dithered v1 class layer"), "{err}");
    }

    #[test]
    fn coarse_heights_are_area_means_and_zero_is_a_code() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, _| {
            (if x == 64 { 4.0 } else { -0.3 }, f32::NAN, 0)
        });
        let out = dir.path().join("export");
        heightmap(
            &world,
            &out,
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
        )
        .unwrap();
        let m: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        let (h_min, step) = (
            m["encoding"]["h_min_m"].as_f64().unwrap(),
            m["encoding"]["step_m"].as_f64().unwrap(),
        );
        let k = -h_min / step;
        assert!((k - k.round()).abs() < 1e-6, "{k}");
        assert!(h_min <= -0.3 && m["encoding"]["h_max_m"].as_f64().unwrap() >= 4.0);
        // L1 sample 32 is column 0 of tile 2.
        let (w, _, t) = view::read_png(&out.join("height/L1/height_L1_2_3.png")).unwrap();
        for y in 0..w {
            let code = u16::from_be_bytes([t[2 * y * w], t[2 * y * w + 1]]);
            let height = h_min + f64::from(code) * step;
            assert!((height - 1.85).abs() <= step, "{height}");
        }
    }

    fn manifest(out: &Path) -> Value {
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap()
    }

    #[test]
    fn partial_export_rebuilds_named_groups_and_links_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 256, 1.0, |x, y| {
            if (64..128).contains(&x) && (64..128).contains(&y) {
                (5.0, f32::NAN, 2)
            } else {
                ((x + y) as f32 * 0.1, f32::NAN, 0)
            }
        });
        testworld::game(&world, 1, |x, _| testworld::Game {
            biome: [1, 2, 200],
            land_cover: [30, 10, 160],
            densities: [x as u8, 10, 20, 150, 100],
        });
        let first = dir.path().join("a");
        heightmap(
            &world,
            &first,
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
        )
        .unwrap();
        // Change the vegetation layer only.
        testworld::game(&world, 1, |x, _| testworld::Game {
            biome: [1, 2, 200],
            land_cover: [30, 10, 160],
            densities: [255 - x as u8, 10, 20, 150, 100],
        });
        let run = |out: &str, layers: &[&str]| {
            heightmap_from(
                &world,
                &dir.path().join(out),
                16,
                Layout::SharedEdge,
                false,
                NormalOptions::default(),
                &Partial {
                    from: first.clone(),
                    layers: layers.iter().map(|s| (*s).to_owned()).collect(),
                },
            )
            .map_err(|e| e.to_string())
        };
        // Reusing the changed layer fails, naming it.
        let err = run("b", &["normals"]).unwrap_err();
        assert!(err.contains("layer group vegetation-density"), "{err}");
        run("c", &["vegetation-density"]).unwrap();
        let second = dir.path().join("c");
        let m = manifest(&second);
        assert_eq!(m["validation"]["passed"], true);
        let sha = digest(&fs::read(first.join("manifest.json")).unwrap());
        assert_eq!(
            m["reused_layers"]["terrain"],
            json!({"name": "a", "manifest_sha256": sha})
        );
        assert!(m["reused_layers"].get("vegetation-density").is_none());
        assert_eq!(m["layers"]["height"], manifest(&first)["layers"]["height"]);
        assert_eq!(m["description"], manifest(&first)["description"]);
        // Reused tiles are hardlinks; rebuilt ones differ; checksums cover both.
        use std::os::unix::fs::MetadataExt;
        let tile = |root: &Path, p: &str| fs::metadata(root.join(p)).unwrap().ino();
        let h = "height/L0/height_L0_0_0.png";
        assert_eq!(tile(&first, h), tile(&second, h));
        let v = "vegetation-density/L1/vegetation-density_L1_0_0.png";
        assert_ne!(
            fs::read(first.join(v)).unwrap(),
            fs::read(second.join(v)).unwrap()
        );
        let sums = fs::read_to_string(second.join("checksums.txt")).unwrap();
        for line in sums.lines() {
            let (sha, path) = line.split_once("  ").unwrap();
            assert_eq!(digest(&fs::read(second.join(path)).unwrap()), sha, "{path}");
        }
        let count = |root: &Path| {
            fs::read_to_string(root.join("checksums.txt"))
                .unwrap()
                .lines()
                .count()
        };
        assert_eq!(count(&first), count(&second));
        let report: Value =
            serde_json::from_slice(&fs::read(second.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["reused_from"]["passed"], true);
        // A chained partial export from the partial one works too.
        heightmap_from(
            &world,
            &dir.path().join("d"),
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
            &Partial {
                from: second.clone(),
                layers: vec!["description".into()],
            },
        )
        .unwrap();
        // Exports without input digests cannot be reused.
        let mut old = manifest(&first);
        old.as_object_mut().unwrap().remove("input_digests");
        let legacy = dir.path().join("legacy");
        link_tree(&first, &legacy).unwrap();
        fs::remove_file(legacy.join("manifest.json")).unwrap();
        fs::write(legacy.join("manifest.json"), old.to_string()).unwrap();
        let err = heightmap_from(
            &world,
            &dir.path().join("e"),
            16,
            Layout::SharedEdge,
            false,
            NormalOptions::default(),
            &Partial {
                from: legacy,
                layers: vec!["vegetation-density".into()],
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("records no input digests"), "{err}");
    }
}
