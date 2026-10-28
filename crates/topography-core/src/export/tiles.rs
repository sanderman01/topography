//! Tile geometry, row windows, apron cutting, PNG writing and the shared-edge check.

use super::checks::Checks;
use super::layers::LayerSpec;
use crate::acquisition::{AcquisitionError, Result, digest, write_new};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub intervals: usize,
    pub shared: bool,
}

impl Geometry {
    pub fn per_tile(self) -> usize {
        self.intervals + usize::from(self.shared)
    }

    pub fn count(self, samples: usize) -> usize {
        samples.div_ceil(self.intervals)
    }

    /// First sample and texel count of tile t on one axis (the last tile is shorter).
    pub fn span(self, t: usize, samples: usize) -> (usize, usize) {
        let first = t * self.intervals;
        (first, self.per_tile().min(samples - first))
    }
}

/// Collects one level's rows of one layer and hands out complete rows of tiles: the rows
/// [ty·N − margin, ty·N + th + margin), clamped to the world, where margin = apron + extra.
pub(super) struct RowWindow<T: Clone> {
    geometry: Geometry,
    samples: usize,
    margin: usize,
    rows: VecDeque<(usize, T)>,
    next_ty: usize,
}

impl<T: Clone> RowWindow<T> {
    pub fn new(geometry: Geometry, samples: usize, margin: usize) -> Self {
        // Each pushed row completes at most one tile row.
        debug_assert!(margin < geometry.intervals);
        Self {
            geometry,
            samples,
            margin,
            rows: VecDeque::new(),
            next_ty: 0,
        }
    }

    /// First row the window holds for tile row `ty`.
    fn start(&self, ty: usize) -> usize {
        (ty * self.geometry.intervals).saturating_sub(self.margin)
    }

    /// Push row y; once complete, returns (ty, first row = ty·N − margin clamped, rows from
    /// there oldest first).
    pub fn push(&mut self, y: usize, row: T) -> Option<(usize, usize, Vec<T>)> {
        debug_assert!(self.rows.back().is_none_or(|(last, _)| *last + 1 == y));
        let ty = self.next_ty;
        if ty >= self.geometry.count(self.samples) {
            return None;
        }
        self.rows.push_back((y, row));
        let (y0, th) = self.geometry.span(ty, self.samples);
        if y + 1 < (y0 + th + self.margin).min(self.samples) {
            return None;
        }
        self.next_ty += 1;
        // Rows the next tile row still needs are cloned, the rest moved out.
        let keep = self.start(ty + 1);
        let mut out = Vec::with_capacity(self.rows.len());
        while self.rows.front().is_some_and(|(r, _)| *r < keep) {
            out.push(self.rows.pop_front().expect("front exists").1);
        }
        out.extend(self.rows.iter().map(|(_, r)| r.clone()));
        Some((ty, self.start(ty), out))
    }
}

/// Cuts tile tx of a row window: columns [x0 − apron, x0 + tw + apron), clamped.
pub(super) fn cut<T: Copy>(
    rows: &[&[T]],
    geometry: Geometry,
    samples: usize,
    tx: usize,
    apron: usize,
) -> Vec<T> {
    let (x0, tw) = geometry.span(tx, samples);
    let w = tw + 2 * apron;
    let mut out = Vec::with_capacity(w * rows.len());
    for row in rows {
        out.extend((0..w).map(|c| row[(x0 + c).saturating_sub(apron).min(samples - 1)]));
    }
    out
}

/// Edges kept from written tiles for their east and south neighbours.
#[derive(Default)]
struct EdgeMemo {
    /// (tx, ty, rightmost columns row-major) of the last tile written.
    right: Option<(usize, usize, Vec<u8>)>,
    /// tx → (ty, bottom rows) of the last tile written in that column.
    bottom: HashMap<usize, (usize, Vec<u8>)>,
}

/// Writes PNG tiles into the stage, keeps checksums.txt lines and compares shared edges
/// and aprons with each tile's west and north neighbours.
pub(super) struct TileWriter {
    stage: PathBuf,
    shared: bool,
    checksums: String,
    edges: HashMap<(&'static str, u32), EdgeMemo>,
}

impl TileWriter {
    pub fn new(stage: &Path, shared: bool) -> Self {
        Self {
            stage: stage.into(),
            shared,
            checksums: String::new(),
            edges: HashMap::new(),
        }
    }

    /// `texels` is row-major with side `w` × `h` including the apron; 8- or 16-bit big endian.
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &mut self,
        spec: &LayerSpec,
        level: u32,
        tx: usize,
        ty: usize,
        w: usize,
        h: usize,
        texels: &[u8],
        checks: &mut Checks,
    ) -> Result<()> {
        let bytes = if spec.sixteen_bit { 2 } else { 1 };
        debug_assert_eq!(texels.len(), w * h * bytes);
        let path = super::tile_path(spec.name, level, tx, ty);
        let hash = png(&self.stage.join(&path), w, h, spec.sixteen_bit, texels)?;
        self.checksums.push_str(&format!("{hash}  {path}\n"));

        // Neighbours overlap by both aprons plus the shared column.
        let k = (2 * spec.apron + usize::from(self.shared)).min(w).min(h);
        let memo = self.edges.entry((spec.name, level)).or_default();
        let (kb, rb) = (k * bytes, w * bytes);
        let left: Vec<u8> = texels.chunks(rb).flat_map(|r| &r[..kb]).copied().collect();
        let right: Vec<u8> = texels
            .chunks(rb)
            .flat_map(|r| &r[rb - kb..])
            .copied()
            .collect();
        let mut compare = |kept: &[u8], now: &[u8], side: &str| {
            let bad = kept
                .chunks(bytes)
                .zip(now.chunks(bytes))
                .filter(|(a, b)| a != b)
                .count() as u64;
            if bad > 0 || kept.len() != now.len() {
                checks.shared_edge_mismatches += bad.max(1);
                checks.example("shared_edge_mismatches", || {
                    format!("{path}: {bad} texels differ from the {side} tile")
                });
            }
        };
        if k > 0 && tx > 0 {
            if let Some((px, py, kept)) = &memo.right {
                if (*px, *py) == (tx - 1, ty) {
                    compare(kept, &left, "west");
                }
            }
        }
        if k > 0 && ty > 0 {
            if let Some((py, kept)) = memo.bottom.get(&tx) {
                if *py == ty - 1 {
                    compare(kept, &texels[..k * rb], "north");
                }
            }
        }
        memo.right = Some((tx, ty, right));
        memo.bottom
            .insert(tx, (ty, texels[(h - k) * rb..].to_vec()));
        Ok(())
    }

    /// Writes checksums.txt.
    pub fn finish(self) -> Result<()> {
        write_new(&self.stage.join("checksums.txt"), self.checksums.as_bytes())
    }
}

fn png(path: &Path, width: usize, height: usize, sixteen: bool, data: &[u8]) -> Result<String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(if sixteen {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_hands_out_overlapping_tile_rows() {
        let g = Geometry {
            intervals: 4,
            shared: true,
        };
        let mut w = RowWindow::new(g, 12, 2);
        let mut got = Vec::new();
        for y in 0..12 {
            if let Some((ty, first, rows)) = w.push(y, y) {
                assert_eq!(rows[0], first);
                got.push((y, ty, rows));
            }
        }
        // Tile rows need [0, 7), [2, 11) and [6, 12).
        assert_eq!(got[0], (6, 0, (0..7).collect()));
        assert_eq!(got[1], (10, 1, (2..11).collect()));
        assert_eq!(got[2], (11, 2, (6..12).collect()));
    }

    #[test]
    fn writer_counts_texels_that_differ_from_the_west_apron() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = TileWriter::new(dir.path(), true);
        let mut checks = Checks::default();
        let spec = super::super::layers::BIOME;
        // Side 3 + 2 * 4 = 11; the 9 overlap columns of 11 rows must match.
        let west: Vec<u8> = (0..121).map(|i| (i % 11) as u8).collect();
        let east: Vec<u8> = (0..121).map(|i| (i % 11 + 2) as u8).collect();
        let mut far: Vec<u8> = (0..121).map(|i| (i % 11 + 4) as u8).collect();
        far[11] = 0;
        writer
            .write(&spec, 1, 0, 0, 11, 11, &west, &mut checks)
            .unwrap();
        writer
            .write(&spec, 1, 1, 0, 11, 11, &east, &mut checks)
            .unwrap();
        assert_eq!(checks.shared_edge_mismatches, 0);
        writer
            .write(&spec, 1, 2, 0, 11, 11, &far, &mut checks)
            .unwrap();
        assert_eq!(checks.shared_edge_mismatches, 1);
        writer.finish().unwrap();
        let sums = std::fs::read_to_string(dir.path().join("checksums.txt")).unwrap();
        assert_eq!(sums.lines().count(), 3);
    }

    #[test]
    fn cut_clamps_aprons_at_the_world_edge() {
        let g = Geometry {
            intervals: 4,
            shared: false,
        };
        let row: Vec<u8> = (0..8).collect();
        assert_eq!(cut(&[&row], g, 8, 0, 2), [0, 0, 0, 1, 2, 3, 4, 5]);
        assert_eq!(cut(&[&row], g, 8, 1, 2), [2, 3, 4, 5, 6, 7, 7, 7]);
    }
}
