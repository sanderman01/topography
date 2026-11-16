//! Baked terrain normal (BC5) and Toksvig roughness (BC4) tiles as KTX2. Every level-0 cell
//! (between four ground samples) gets a unit normal; a level-k texel is the box mean of the
//! 2^k × 2^k cell normals it covers, built once globally and cut into cell-centred tiles.

use super::bc;
use super::checks::Checks;
use super::ktx2::{self, Format};
use super::tiles::{Edges, Geometry, RowWindow, TileWriter};
use crate::acquisition::{Result, ensure};
use serde_json::{Map, Value, json};

/// Normal and roughness export settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalOptions {
    pub enabled: bool,
    /// First level with tiles; the game computes finer normals from the height tiles.
    pub start_level: u32,
    /// Texels repeated from the neighbours on every side; even, so tiles are whole blocks.
    pub apron: usize,
    pub roughness: bool,
}

impl Default for NormalOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            start_level: 2,
            // Two texels keep tiles whole 4×4 BC blocks and let filtering cross tile borders.
            apron: 2,
            roughness: false,
        }
    }
}

impl NormalOptions {
    pub(super) fn validate(&self, intervals: usize, last: u32) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        ensure(
            self.start_level <= last,
            format!(
                "normal start level {} is beyond the export's last level {last}",
                self.start_level
            ),
        )?;
        ensure(
            (intervals + 2 * self.apron) % 4 == 0,
            "normal tiles must be whole 4×4 BC blocks: intervals + 2·apron must be a multiple of 4 (use an even apron)",
        )?;
        ensure(
            self.apron < intervals,
            "normal apron must be smaller than intervals",
        )
    }
}

/// Fails the export: BC5 normal error above this angle, in degrees. Real 1 m terrain reaches
/// about 20° on a few steep high-contrast blocks (z is rebuilt from x and y), so only larger
/// errors point at a bug.
const MAX_ANGLE_DEG: f64 = 30.0;
/// Texels above this angle are counted in the budget.
const NOTE_ANGLE_DEG: f64 = 10.0;
/// Fails the export: BC4 roughness error above this.
const MAX_ROUGHNESS_ERROR: f64 = 0.25;
const ANGLE_BIN_DEG: f64 = 0.001;
const ANGLE_BINS: usize = 20_000;
const ROUGH_BIN: f64 = 1e-4;
const ROUGH_BINS: usize = 10_000;

type Vec3 = [f32; 3];

/// Unit normals (x east, y north, z up) of the cells between ground rows `north` and
/// `south`: the gradient across each cell from its four corner samples.
fn cell_row(north: &[f32], south: &[f32], spacing: f32) -> Vec<Vec3> {
    let k = 0.5 / spacing;
    (0..north.len() - 1)
        .map(|x| {
            let (a, b, c, d) = (north[x], north[x + 1], south[x], south[x + 1]);
            let east = ((b - a) + (d - c)) * k;
            let up_north = ((a - c) + (b - d)) * k;
            let inv = 1.0 / (east * east + up_north * up_north + 1.0).sqrt();
            [-east * inv, -up_north * inv, inv]
        })
        .collect()
}

/// Box mean of 2×2 texels from rows `a` and `b`; an odd last column is dropped.
fn reduce(a: &[Vec3], b: &[Vec3]) -> Vec<Vec3> {
    (0..a.len() / 2)
        .map(|j| {
            let (p, q, r, s) = (a[2 * j], a[2 * j + 1], b[2 * j], b[2 * j + 1]);
            std::array::from_fn(|k| (p[k] + q[k] + r[k] + s[k]) * 0.25)
        })
        .collect()
}

/// Normalized mean normal and Toksvig roughness: m = |mean|, σ = sqrt((1 − m) / m) ≤ 1.
fn texel(mean: Vec3) -> (Vec3, f32) {
    let m = (mean[0] * mean[0] + mean[1] * mean[1] + mean[2] * mean[2]).sqrt();
    let n = mean.map(|v| v / m);
    let sigma = ((1.0 - m).max(0.0) / m).sqrt().min(1.0);
    (n, sigma)
}

/// Error histograms of one level.
#[derive(Clone)]
struct Stats {
    angle: Vec<u64>,
    angle_max: f64,
    rough: Vec<u64>,
    rough_max: f64,
    texels: u64,
    over_note: u64,
    gross: u64,
    /// (x, y, angle °, roughness error) of the first gross texel.
    example: Option<(usize, usize, f64, f64)>,
}

impl Stats {
    fn new() -> Self {
        Self {
            angle: vec![0; ANGLE_BINS + 1],
            angle_max: 0.0,
            rough: vec![0; ROUGH_BINS + 1],
            rough_max: 0.0,
            texels: 0,
            over_note: 0,
            gross: 0,
            example: None,
        }
    }

    fn add(&mut self, (x, y): (usize, usize), angle: f64, rough: f64) {
        self.texels += 1;
        self.angle[((angle / ANGLE_BIN_DEG) as usize).min(ANGLE_BINS)] += 1;
        self.angle_max = self.angle_max.max(angle);
        self.rough[((rough / ROUGH_BIN) as usize).min(ROUGH_BINS)] += 1;
        self.rough_max = self.rough_max.max(rough);
        self.over_note += u64::from(angle > NOTE_ANGLE_DEG);
        if angle > MAX_ANGLE_DEG || rough > MAX_ROUGHNESS_ERROR {
            self.gross += 1;
            self.example.get_or_insert((x, y, angle, rough));
        }
    }

    fn merge(&mut self, other: &Self) {
        for (a, b) in self.angle.iter_mut().zip(&other.angle) {
            *a += b;
        }
        for (a, b) in self.rough.iter_mut().zip(&other.rough) {
            *a += b;
        }
        self.angle_max = self.angle_max.max(other.angle_max);
        self.rough_max = self.rough_max.max(other.rough_max);
        self.texels += other.texels;
        self.over_note += other.over_note;
        self.gross += other.gross;
        if self.example.is_none() {
            self.example = other.example;
        }
    }

    /// Upper edge of the bin holding the 99th percentile.
    fn p99(hist: &[u64], bin: f64, total: u64) -> f64 {
        let target = (total as f64 * 0.99).ceil() as u64;
        let mut seen = 0;
        for (i, &n) in hist.iter().enumerate() {
            seen += n;
            if seen >= target {
                return (i + 1) as f64 * bin;
            }
        }
        hist.len() as f64 * bin
    }

    fn report(&self, roughness: bool) -> Value {
        let round = |v: f64| (v * 1e4).round() / 1e4;
        let mut r = json!({
            "texels": self.texels,
            "angle_p99_deg": round(Self::p99(&self.angle, ANGLE_BIN_DEG, self.texels)),
            "angle_max_deg": round(self.angle_max),
            "angle_over_10_deg": self.over_note,
        });
        if roughness {
            r["roughness_p99_abs"] = json!(round(Self::p99(&self.rough, ROUGH_BIN, self.texels)));
            r["roughness_max_abs"] = json!(round(self.rough_max));
        }
        r
    }
}

/// A KTX2 file and its decoded texels (f32 LE per channel) for the seam check.
struct Encoded {
    file: Vec<u8>,
    texels: Vec<u8>,
}

struct Tile {
    tx: usize,
    size: (usize, usize),
    normal: Encoded,
    rough: Option<Encoded>,
}

/// Encodes tile `tx` from columns `cols` of `rows` (both padded), measuring the error over
/// the valid texels `[a, a + tw) × [a, a + th)` whose level texel is (x0, y0) at (a, a).
fn bake(
    tx: usize,
    rows: &[&[Vec3]],
    cols: &[usize],
    roughness: bool,
    (a, tw, th): (usize, usize, usize),
    (x0, y0): (usize, usize),
    stats: &mut Stats,
) -> Tile {
    let (w, h) = (cols.len(), rows.len());
    let mut normal = Vec::with_capacity(w * h);
    let mut rough = Vec::with_capacity(w * h);
    for row in rows {
        for &c in cols {
            normal.push(texel(row[c]));
        }
    }
    let red: Vec<f32> = normal.iter().map(|(n, _)| 0.5 * (n[0] + 1.0)).collect();
    let green: Vec<f32> = normal.iter().map(|(n, _)| 0.5 * (n[1] + 1.0)).collect();
    rough.extend(normal.iter().map(|(_, s)| *s));
    let blocks = bc::bc5(&red, &green, w, h);
    let planes = bc::decode(&blocks, w, h, 2);
    let rough_blocks = roughness.then(|| bc::bc4(&rough, w, h));
    let rough_decoded = rough_blocks
        .as_ref()
        .map(|b| bc::decode(b, w, h, 1).remove(0));
    for r in a..a + th {
        for c in a..a + tw {
            let i = r * w + c;
            let n = normal[i].0.map(f64::from);
            let dx = 2.0 * f64::from(planes[0][i]) - 1.0;
            let dy = 2.0 * f64::from(planes[1][i]) - 1.0;
            let dz = (1.0 - dx * dx - dy * dy).max(0.0).sqrt();
            let cross = [
                n[1] * dz - n[2] * dy,
                n[2] * dx - n[0] * dz,
                n[0] * dy - n[1] * dx,
            ];
            let sin = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
            let angle = sin.atan2(n[0] * dx + n[1] * dy + n[2] * dz).to_degrees();
            let rough_error = rough_decoded
                .as_ref()
                .map_or(0.0, |d| f64::from((d[i] - rough[i]).abs()));
            stats.add((x0 + c - a, y0 + r - a), angle, rough_error);
        }
    }
    let normal_texels = planes[0]
        .iter()
        .zip(&planes[1])
        .flat_map(|(r, g)| r.to_le_bytes().into_iter().chain(g.to_le_bytes()))
        .collect();
    let rough = rough_blocks.map(|b| Encoded {
        file: ktx2::write(Format::Bc4Unorm, w, h, &b),
        texels: rough_decoded
            .unwrap_or_default()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
    });
    Tile {
        tx,
        size: (w, h),
        normal: Encoded {
            file: ktx2::write(Format::Bc5Unorm, w, h, &blocks),
            texels: normal_texels,
        },
        rough,
    }
}

/// One pyramid level's rolling state.
struct Level {
    held: Option<Vec<Vec3>>,
    next_y: usize,
}

/// Streams level-0 ground rows into cell normals, the box pyramid and tiles.
pub(super) struct Baker {
    options: NormalOptions,
    spacing: f32,
    geometry: Geometry,
    last: u32,
    north: Option<Vec<f32>>,
    pyramid: Vec<Level>,
    /// Levels start..=last.
    windows: Vec<RowWindow<Vec<Vec3>>>,
    stats: Vec<Stats>,
    /// Level-0 samples per side.
    world: usize,
    threads: usize,
}

impl Baker {
    pub fn new(
        options: NormalOptions,
        intervals: usize,
        world: usize,
        spacing: f64,
        last: u32,
    ) -> Self {
        let geometry = Geometry {
            intervals,
            shared: false,
        };
        let start = options.start_level;
        Self {
            options,
            spacing: spacing as f32,
            geometry,
            last,
            north: None,
            pyramid: (0..=last)
                .map(|_| Level {
                    held: None,
                    next_y: 0,
                })
                .collect(),
            windows: (start..=last)
                .map(|l| RowWindow::new(geometry, cells(world, l), options.apron))
                .collect(),
            stats: (start..=last).map(|_| Stats::new()).collect(),
            world,
            threads: std::thread::available_parallelism().map_or(1, |n| n.get().min(8)),
        }
    }

    /// Feeds level-0 ground row y (rows in order from 0).
    pub fn push(
        &mut self,
        ground: &[f32],
        writer: &mut TileWriter,
        checks: &mut Checks,
    ) -> Result<()> {
        let Some(north) = self.north.replace(ground.to_vec()) else {
            return Ok(());
        };
        let mut row = cell_row(&north, ground, self.spacing);
        for level in 0..=self.last {
            let y = self.pyramid[level as usize].next_y;
            self.pyramid[level as usize].next_y += 1;
            if level >= self.options.start_level {
                let window = &mut self.windows[(level - self.options.start_level) as usize];
                if let Some((ty, first, rows)) = window.push(y, row.clone()) {
                    self.tile_row(level, ty, first, &rows, writer, checks)?;
                }
            }
            if level == self.last {
                break;
            }
            let state = &mut self.pyramid[level as usize];
            if y % 2 == 0 {
                state.held = Some(row);
                break;
            }
            let held = state.held.take().expect("even row precedes odd row");
            row = reduce(&held, &row);
        }
        Ok(())
    }

    /// Cuts, encodes (in parallel) and writes tile row `ty` of `level`; `rows` start at
    /// cell row `first`.
    fn tile_row(
        &mut self,
        level: u32,
        ty: usize,
        first: usize,
        rows: &[Vec<Vec3>],
        writer: &mut TileWriter,
        checks: &mut Checks,
    ) -> Result<()> {
        let (g, a, n) = (self.geometry, self.options.apron, cells(self.world, level));
        let (y0, th) = g.span(ty, n);
        // Past the world edge (apron and block padding) the edge texel repeats.
        let h = (th + 2 * a).next_multiple_of(4);
        let picked: Vec<&[Vec3]> = (0..h)
            .map(|r| rows[(y0 + r).saturating_sub(a).min(n - 1) - first].as_slice())
            .collect();
        let count = g.count(n);
        let roughness = self.options.roughness;
        let threads = self.threads.min(count);
        let picked = &picked;
        let results: Vec<(Vec<Tile>, Stats)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    s.spawn(move || {
                        let mut stats = Stats::new();
                        let tiles = (t..count)
                            .step_by(threads)
                            .map(|tx| {
                                let (x0, tw) = g.span(tx, n);
                                let w = (tw + 2 * a).next_multiple_of(4);
                                let cols: Vec<usize> = (0..w)
                                    .map(|c| (x0 + c).saturating_sub(a).min(n - 1))
                                    .collect();
                                bake(
                                    tx,
                                    picked,
                                    &cols,
                                    roughness,
                                    (a, tw, th),
                                    (x0, y0),
                                    &mut stats,
                                )
                            })
                            .collect();
                        (tiles, stats)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("tile encoder thread panicked"))
                .collect()
        });
        let stats = &mut self.stats[(level - self.options.start_level) as usize];
        let mut tiles = Vec::with_capacity(count);
        for (t, s) in results {
            stats.merge(&s);
            tiles.extend(t);
        }
        tiles.sort_by_key(|t| t.tx);
        for tile in tiles {
            let mut files = vec![("normal", tile.normal, 8)];
            if let Some(rough) = tile.rough {
                files.push(("roughness", rough, 4));
            }
            for (layer, Encoded { file, texels }, bytes) in files {
                let path = super::tile_file(layer, level, tile.tx, ty, "ktx2");
                writer.write_file(&path, &file)?;
                let edges = Edges {
                    layer,
                    level,
                    tile: (tile.tx, ty),
                    size: tile.size,
                    bytes,
                    overlap: 2 * a,
                };
                writer.compare_edges(&edges, &texels, &path, checks);
            }
        }
        Ok(())
    }

    /// Records the error budget and gross errors in `checks` and returns the manifest
    /// metadata of the `normal` and `roughness` layers.
    pub fn finish(self, checks: &mut Checks, level0_spacing: f64) -> Map<String, Value> {
        let start = self.options.start_level;
        let mut budget = Map::new();
        for (i, s) in self.stats.iter().enumerate() {
            let level = start + i as u32;
            budget.insert(format!("L{level}"), s.report(self.options.roughness));
            checks.bc_gross_errors += s.gross;
            if let Some((x, y, angle, rough)) = s.example {
                checks.example("bc_gross_errors", || {
                    format!("L{level} texel ({x}, {y}): normal {angle:.2}°, roughness {rough:.3}")
                });
            }
        }
        checks.bc_budget = Some(json!({
            "fail_above": {"angle_deg": MAX_ANGLE_DEG, "roughness_abs": MAX_ROUGHNESS_ERROR},
            "levels": budget,
        }));
        let (g, a) = (self.geometry, self.options.apron);
        let levels: Vec<Value> = (start..=self.last)
            .map(|level| {
                let n = cells(self.world, level);
                let count = g.count(n);
                let valid = g.span(count - 1, n).1;
                json!({"level": level, "spacing_m": level0_spacing * f64::from(1u32 << level),
                    "cells_per_side": n, "tiles_per_side": count,
                    "tile_texels": g.intervals + 2 * a,
                    "last_tile_valid_texels": valid,
                    "last_tile_texels": (valid + 2 * a).next_multiple_of(4)})
            })
            .collect();
        let common = |format: Format, name: &str, decode: &str, unit: &str, filter: &str| {
            json!({
                "prefix": name,
                "format": format!("KTX2 {}", format.name()),
                "vk_format": format.vk_format(),
                "file_extension": "ktx2",
                "mip_levels": 1,
                "supercompression": "none",
                "decode": decode,
                "unit": unit,
                "finest_spacing_m": level0_spacing * f64::from(1u32 << start),
                "levels": [start, self.last],
                "filter": filter,
                "source": "generated",
                "apron_texels": a,
                "missing": "never missing",
                "texel_lattice": "cell-centred: texel (i, j) of level k covers the level-k cell between height samples i and i+1 (columns) and j and j+1 (rows), i.e. level-0 samples [i·2^k, (i+1)·2^k]; sample it at ((i + 0.5) / N) to hit the cell centre. Tile (x, y) texel (A, A) is cell (x·N, y·N), N = intervals, A = apron_texels",
                "tile_size": "N + 2·A texels per side; the last tile of each row and column holds last_tile_valid_texels cells plus aprons, padded to a multiple of 4 by repeating the edge texel",
                "row_order": "KTXorientation rd: row 0 is the north edge, column 0 the west edge",
                "tile_levels": levels,
            })
        };
        let mut out = Map::new();
        out.insert(
            "normal".into(),
            common(
                Format::Bc5Unorm,
                "normal",
                "x = 2·R − 1, y = 2·G − 1, z = sqrt(max(0, 1 − x² − y²)); normalize(x, y, z)",
                "unit world-space normal: x east, y north, z up",
                "normalized box mean of the level-0 cell normals under the texel; a cell normal is the gradient across the cell from its four corner ground samples",
            ),
        );
        if self.options.roughness {
            out.insert(
                "roughness".into(),
                common(
                    Format::Bc4Unorm,
                    "roughness",
                    "sigma = R",
                    "Toksvig slope deviation, 0 for a plane",
                    "m = |box mean of the level-0 cell normals|; sigma = min(sqrt((1 − m) / m), 1)",
                ),
            );
        }
        out
    }
}

/// Cells per side of `level`: one fewer than its height samples.
fn cells(world: usize, level: u32) -> usize {
    (world >> level) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mean vectors of every level from a ground function on an n² world.
    fn levels(n: usize, last: u32, z: impl Fn(usize, usize) -> f32) -> Vec<Vec<Vec<Vec3>>> {
        let rows: Vec<Vec<f32>> = (0..n).map(|y| (0..n).map(|x| z(x, y)).collect()).collect();
        let mut out: Vec<Vec<Vec<Vec3>>> = vec![
            rows.windows(2)
                .map(|p| cell_row(&p[0], &p[1], 1.0))
                .collect(),
        ];
        for _ in 0..last {
            let fine = out.last().unwrap();
            let coarse = (0..fine.len() / 2)
                .map(|i| reduce(&fine[2 * i], &fine[2 * i + 1]))
                .collect();
            out.push(coarse);
        }
        out
    }

    #[test]
    fn flat_plane_is_up_with_zero_roughness() {
        for level in levels(16, 3, |_, _| 42.0) {
            for t in level.iter().flatten() {
                assert_eq!(texel(*t), ([0.0, 0.0, 1.0], 0.0));
            }
        }
    }

    #[test]
    fn tilted_plane_keeps_its_normal_at_every_level() {
        // z rises 0.5 m per metre east and falls 0.25 m per row south, i.e. rises north.
        let z = |x: usize, y: usize| 0.5 * x as f32 - 0.25 * y as f32;
        let len = (0.25f32 + 0.0625 + 1.0).sqrt();
        let expect = [-0.5 / len, -0.25 / len, 1.0 / len];
        let all = levels(32, 4, z);
        for (k, level) in all.iter().enumerate() {
            assert_eq!(level.len(), (32 >> k) - 1);
            for t in level.iter().flatten() {
                let (n, s) = texel(*t);
                for i in 0..3 {
                    assert!((n[i] - expect[i]).abs() < 1e-6, "{n:?}");
                }
                assert!(s < 1e-3, "{s}");
            }
        }
    }

    #[test]
    fn ridges_and_noise_are_rough_when_averaged() {
        // A 1 m sawtooth ridge: cell normals alternate, their mean is up but shorter.
        let saw = levels(32, 2, |x, _| if x % 2 == 0 { 0.0 } else { 1.0 });
        let (n, s) = texel(saw[2][3][3]);
        assert!(n[2] > 0.999 && s > 0.5, "{n:?} {s}");
        let mut seed = 7u32;
        let noise: Vec<f32> = (0..64 * 64)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32
            })
            .collect();
        let rough = levels(64, 3, |x, y| noise[y * 64 + x]);
        assert!(texel(rough[0][5][5]).1 < 1e-3);
        assert!(texel(rough[3][2][2]).1 > 0.1);
    }

    #[test]
    fn options_are_validated() {
        let o = NormalOptions::default();
        assert!(o.validate(256, 8).is_ok());
        assert!(o.validate(256, 1).is_err());
        let odd = NormalOptions { apron: 3, ..o };
        assert!(
            odd.validate(256, 8)
                .unwrap_err()
                .to_string()
                .contains("multiple of 4")
        );
        let two = NormalOptions { apron: 2, ..o };
        assert!(two.validate(256, 8).is_ok());
        let huge = NormalOptions { apron: 256, ..o };
        assert!(huge.validate(256, 8).is_err());
        let off = NormalOptions {
            enabled: false,
            apron: 3,
            ..o
        };
        assert!(off.validate(256, 0).is_ok());
    }

    /// (width, height, red, green-or-empty) of a KTX2 tile.
    fn read_tile(path: &std::path::Path) -> (usize, usize, Vec<Vec<f32>>) {
        let bytes = std::fs::read(path).unwrap();
        let r = ::ktx2::Reader::new(&bytes[..]).unwrap();
        let h = r.header();
        let (w, ht) = (h.pixel_width as usize, h.pixel_height as usize);
        let channels = if h.format == Some(::ktx2::Format::BC5_UNORM_BLOCK) {
            2
        } else {
            1
        };
        let data = r.levels().next().unwrap().data;
        (w, ht, bc::decode(data, w, ht, channels))
    }

    fn export(
        n: usize,
        options: NormalOptions,
        z: impl Fn(usize, usize) -> f32,
    ) -> (tempfile::TempDir, std::path::PathBuf, Value) {
        use super::super::{Layout, heightmap, testworld};
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), n, 1.0, |x, y| (z(x, y), f32::NAN, 0));
        let out = dir.path().join("export");
        heightmap(&world, &out, 16, Layout::SharedEdge, false, options).unwrap();
        let m: Value =
            serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
        (dir, out, m)
    }

    #[test]
    fn tilted_world_exports_its_normal_at_every_level() {
        let options = NormalOptions {
            start_level: 1,
            roughness: true,
            ..NormalOptions::default()
        };
        let (_dir, out, m) = export(128, options, |x, y| 0.5 * x as f32 - 0.25 * y as f32);
        let len = (0.25f32 + 0.0625 + 1.0).sqrt();
        let expect = [-0.5 / len, -0.25 / len];
        let layer = &m["layers"]["normal"];
        assert_eq!(layer["vk_format"], 141);
        assert_eq!(layer["levels"], json!([1, 3]));
        assert_eq!(m["layers"]["roughness"]["vk_format"], 139);
        // L1: 63 cells in 4 tiles of 16; the last holds 15 cells, padded to 20 with aprons.
        let l1 = &layer["tile_levels"][0];
        assert_eq!(
            (&l1["cells_per_side"], &l1["tiles_per_side"]),
            (&json!(63), &json!(4))
        );
        assert_eq!(
            (&l1["last_tile_valid_texels"], &l1["last_tile_texels"]),
            (&json!(15), &json!(20))
        );
        for level in 1..=3 {
            let tiles = (64usize >> (level - 1)).div_ceil(16);
            for (tx, ty) in [(0, 0), (tiles - 1, tiles - 1)] {
                let p = out.join(super::super::tile_file("normal", level, tx, ty, "ktx2"));
                let (w, h, planes) = read_tile(&p);
                assert_eq!((w % 4, h % 4), (0, 0));
                for (r, g) in planes[0].iter().zip(&planes[1]) {
                    assert!((2.0 * r - 1.0 - expect[0]).abs() < 0.01, "{r}");
                    assert!((2.0 * g - 1.0 - expect[1]).abs() < 0.01, "{g}");
                }
                let p = out.join(super::super::tile_file("roughness", level, tx, ty, "ktx2"));
                let (_, _, rough) = read_tile(&p);
                assert!(rough[0].iter().all(|&v| v < 0.01));
            }
        }
        let budget = &m["validation"]["figures"]["bc_error_budget"]["levels"]["L1"];
        assert!(budget["angle_max_deg"].as_f64().unwrap() < 1.0, "{budget}");
        assert_eq!(m["validation"]["checks"]["bc_gross_errors"], 0);
    }

    #[test]
    fn aprons_repeat_the_neighbours_texels() {
        let options = NormalOptions {
            start_level: 0,
            roughness: true,
            ..NormalOptions::default()
        };
        let z = |x: usize, y: usize| 3.0 * (x as f32 / 5.0).sin() + 2.0 * (y as f32 / 7.0).cos();
        let (_dir, out, m) = export(64, options, z);
        assert_eq!(m["validation"]["checks"]["shared_edge_mismatches"], 0);
        let tile = |layer: &str, tx, ty| {
            read_tile(&out.join(super::super::tile_file(layer, 0, tx, ty, "ktx2")))
        };
        for layer in ["normal", "roughness"] {
            // Side 16 + 4; tile (1, 0) columns 0..4 are tile (0, 0) columns 16..20.
            let (w, h, a) = tile(layer, 0, 0);
            let (_, _, b) = tile(layer, 1, 0);
            assert_eq!((w, h), (20, 20));
            for (pa, pb) in a.iter().zip(&b) {
                for r in 0..h {
                    assert_eq!(pa[r * w + 16..r * w + 20], pb[r * w..r * w + 4]);
                }
            }
            let (_, _, c) = tile(layer, 0, 1);
            for (pa, pc) in a.iter().zip(&c) {
                assert_eq!(pa[16 * w..], pc[..4 * w]);
            }
        }
        let (_, _, rough) = tile("roughness", 0, 0);
        // Single 1 m cells of a smooth surface are planes.
        assert!(rough[0].iter().all(|&v| v < 0.01));
    }

    #[test]
    fn coarse_texels_of_a_ridged_world_are_rough() {
        let options = NormalOptions {
            roughness: true,
            ..NormalOptions::default()
        };
        let (_dir, out, m) = export(64, options, |x, _| (x % 2) as f32);
        assert_eq!(m["layers"]["normal"]["apron_texels"], 2);
        let p = out.join(super::super::tile_file("roughness", 2, 0, 0, "ktx2"));
        let (_, _, rough) = read_tile(&p);
        assert!(rough[0].iter().all(|&v| v > 0.5), "{:?}", &rough[0][..4]);
    }

    #[test]
    fn bad_options_fail_before_writing() {
        use super::super::{Layout, heightmap, testworld};
        let dir = tempfile::tempdir().unwrap();
        let world = testworld::write(dir.path(), 64, 1.0, |x, _| (x as f32, f32::NAN, 0));
        let out = dir.path().join("x");
        for (options, text) in [
            (
                NormalOptions {
                    apron: 1,
                    ..NormalOptions::default()
                },
                "multiple of 4",
            ),
            (
                NormalOptions {
                    start_level: 3,
                    ..NormalOptions::default()
                },
                "beyond the export's last level 2",
            ),
        ] {
            let err = heightmap(&world, &out, 16, Layout::SharedEdge, false, options)
                .unwrap_err()
                .to_string();
            assert!(err.contains(text), "{err}");
            assert!(!out.exists());
        }
        let off = NormalOptions {
            enabled: false,
            ..NormalOptions::default()
        };
        heightmap(&world, &out, 16, Layout::SharedEdge, false, off).unwrap();
        assert!(!out.join("normal").exists() && !out.join("roughness").exists());
    }
}
