//! Streaming LOD pyramid: area mean for continuous layers, weighted mode for classes.

use crate::acquisition::Result;

/// Footprint weights of a coarse texel over fine samples 2i-1, 2i, 2i+1.
pub(super) const W: [f32; 3] = [0.25, 0.5, 0.25];

/// Fine sample indices under coarse sample i; -1 clamps to 0.
pub(super) fn taps(i: usize) -> [usize; 3] {
    [(2 * i).saturating_sub(1), 2 * i, 2 * i + 1]
}

/// Builds a level k+1 row from level-k rows 2i-1, 2i, 2i+1.
pub(super) trait Reduce {
    type Row: Clone;
    fn reduce(&self, rows: [&Self::Row; 3]) -> Self::Row;
}

/// Area mean with the [1/4, 1/2, 1/4]² footprint.
pub(super) struct Mean;

impl Reduce for Mean {
    type Row = Vec<f32>;
    fn reduce(&self, [a, b, c]: [&Vec<f32>; 3]) -> Vec<f32> {
        (0..b.len() / 2)
            .map(|i| {
                let t = taps(i);
                [a, b, c]
                    .iter()
                    .zip(W)
                    .map(|(r, wy)| wy * t.iter().zip(W).map(|(&x, wx)| wx * r[x]).sum::<f32>())
                    .sum()
            })
            .collect()
    }
}

/// Weighted majority class over the same footprint, weights 1, 2, 4 (`W` · `W` · 16).
pub(super) struct Mode;

const MODE_W: [u32; 3] = [1, 2, 1];

impl Reduce for Mode {
    type Row = Vec<u8>;
    fn reduce(&self, [a, b, c]: [&Vec<u8>; 3]) -> Vec<u8> {
        let mut weight = [0u32; 256];
        (0..b.len() / 2)
            .map(|i| {
                let t = taps(i);
                let mut seen = Vec::with_capacity(9);
                for (r, wy) in [a, b, c].iter().zip(MODE_W) {
                    for (&x, wx) in t.iter().zip(MODE_W) {
                        let class = r[x];
                        if weight[usize::from(class)] == 0 {
                            seen.push(class);
                        }
                        weight[usize::from(class)] += wy * wx;
                    }
                }
                let best = seen.iter().map(|&k| weight[usize::from(k)]).max();
                let centre = b[2 * i];
                let class = if Some(weight[usize::from(centre)]) == best {
                    centre
                } else {
                    seen.iter()
                        .copied()
                        .filter(|&k| Some(weight[usize::from(k)]) == best)
                        .min()
                        .unwrap_or(centre)
                };
                for k in seen {
                    weight[usize::from(k)] = 0;
                }
                class
            })
            .collect()
    }
}

/// Receives `(level, y, row)`.
pub(super) type Sink<'a, T> = dyn FnMut(u32, usize, &T) -> Result<()> + 'a;

/// One level's rolling state.
struct Level<T> {
    prev_odd: Option<T>,
    held: Option<T>,
    next_y: usize,
}

/// Every level from `first` to `last`, fed one row of level `first` at a time.
pub(super) struct Pyramid<R: Reduce> {
    reduce: R,
    first: u32,
    levels: Vec<Level<R::Row>>,
}

impl<R: Reduce> Pyramid<R> {
    pub fn new(reduce: R, first: u32, last: u32) -> Self {
        assert!(first <= last, "pyramid needs first <= last");
        let levels = (first..=last)
            .map(|_| Level {
                prev_odd: None,
                held: None,
                next_y: 0,
            })
            .collect();
        Self {
            reduce,
            first,
            levels,
        }
    }

    /// `sink(level, y, row)` receives each row of each level once, in row order per level.
    pub fn push(&mut self, row: R::Row, sink: &mut Sink<'_, R::Row>) -> Result<()> {
        let mut row = row;
        let top = self.levels.len() - 1;
        for (k, state) in self.levels.iter_mut().enumerate() {
            let level = self.first + k as u32;
            let y = state.next_y;
            state.next_y += 1;
            sink(level, y, &row)?;
            if k == top {
                break;
            }
            if y % 2 == 0 {
                state.held = Some(row);
                break;
            }
            let held = state.held.take().expect("even row precedes odd row");
            let coarse =
                self.reduce
                    .reduce([state.prev_odd.as_ref().unwrap_or(&held), &held, &row]);
            state.prev_odd = Some(row);
            row = coarse;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run<R: Reduce<Row = Vec<f32>>>(r: R, rows: Vec<Vec<f32>>, last: u32) -> Vec<Vec<Vec<f32>>> {
        let mut levels = vec![Vec::new(); last as usize + 1];
        let mut p = Pyramid::new(r, 0, last);
        for row in rows {
            p.push(row, &mut |l, _, r| {
                levels[l as usize].push(r.clone());
                Ok(())
            })
            .unwrap();
        }
        levels
    }
    #[test]
    fn constant_stays_constant_and_sizes_halve() {
        let l = run(Mean, vec![vec![3.5; 16]; 16], 4);
        for (k, level) in l.iter().enumerate() {
            assert_eq!(level.len(), 16 >> k);
            assert!(level.iter().flatten().all(|v| *v == 3.5));
        }
    }
    #[test]
    fn mean_conserves_interior_mass() {
        // A one-sample-wide line on an odd column keeps its share: 1/4 in two coarse texels.
        let rows = (0..16)
            .map(|_| (0..16).map(|x| f32::from(x == 5)).collect())
            .collect();
        let l = run(Mean, rows, 1);
        assert_eq!(l[1][4][2], 0.25);
        assert_eq!(l[1][4][3], 0.25);
        assert_eq!(l[1][4].iter().sum::<f32>(), 0.5);
    }
    #[test]
    fn coarse_texels_keep_their_positions() {
        // A spike at fine sample (8, 8) is centred on coarse sample (4, 4).
        let rows = (0..16)
            .map(|y| (0..16).map(|x| f32::from(x == 8 && y == 8)).collect())
            .collect();
        let l = run(Mean, rows, 1);
        assert_eq!(l[1][4][4], 0.25);
        assert_eq!(l[1][4][3], 0.0);
    }
    #[test]
    fn mode_takes_weighted_majority_and_centre_on_ties() {
        // Coarse 0 (taps 0, 0, 1): class 1 outweighs class 2.
        let (a, c) = (vec![1u8, 2, 2, 0], vec![1u8, 1, 2, 0]);
        assert_eq!(Mode.reduce([&a, &a, &c])[0], 1);
        // Coarse 1 (taps 1, 2, 3): 4 and 7 weigh the same, so the centre's 7 wins.
        let t = vec![0u8, 4, 7, 4];
        assert_eq!(Mode.reduce([&t, &t, &t])[1], 7);
        // A tie the centre is not part of goes to the lowest id.
        let (u, m, v) = (vec![0u8, 9, 9, 9], vec![0u8, 9, 1, 3], vec![0u8, 3, 3, 3]);
        assert_eq!(Mode.reduce([&u, &m, &v])[1], 3);
    }
    #[test]
    fn sink_sees_rows_in_order_from_first_level() {
        let mut seen = Vec::new();
        let mut p = Pyramid::new(Mode, 2, 4);
        for _ in 0..8 {
            p.push(vec![1u8; 8], &mut |l, y, r| {
                seen.push((l, y, r.len()));
                Ok(())
            })
            .unwrap();
        }
        for (level, rows) in [(2, 8), (3, 4), (4, 2)] {
            let ys: Vec<_> = seen.iter().filter(|s| s.0 == level).collect();
            assert_eq!(ys.len(), rows);
            assert!(ys.iter().enumerate().all(|(i, s)| s.1 == i && s.2 == rows));
        }
    }
}
