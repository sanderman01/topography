//! Streaming LOD pyramid: area mean for continuous layers and for class shares.

use crate::acquisition::Result;

/// Footprint weights of a coarse texel over fine samples 2i-1, 2i, 2i+1.
pub(super) const W: [f32; 3] = [0.25, 0.5, 0.25];

/// Fine sample indices under coarse sample i; -1 clamps to 0.
pub(super) fn taps(i: usize) -> [usize; 3] {
    [(2 * i).saturating_sub(1), 2 * i, 2 * i + 1]
}

/// Footprint mean of coarse texel `i` over level-k rows 2i-1, 2i, 2i+1.
pub(super) fn mean_at(rows: [&[f32]; 3], i: usize) -> f32 {
    let t = taps(i);
    rows.iter()
        .zip(W)
        .map(|(r, wy)| wy * t.iter().zip(W).map(|(&x, wx)| wx * r[x]).sum::<f32>())
        .sum()
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
        debug_assert!(a.len() == b.len() && c.len() == b.len() && b.len() % 2 == 0);
        (0..b.len() / 2).map(|i| mean_at([a, b, c], i)).collect()
    }
}

/// Class shares kept per texel, heaviest first; unused slots have weight 0.
pub(super) const SHARES: usize = 4;
pub(super) type Shares = [(u8, f32); SHARES];

/// Shares of a top-two class texel: `a` with `weight`/255, `b` with the rest.
pub(super) fn shares(a: u8, b: u8, weight: u8) -> Shares {
    let mut s = [(0, 0.0); SHARES];
    let wa = f32::from(weight) / 255.0;
    if a == b || weight == 255 {
        s[0] = (a, 1.0);
    } else {
        s[0] = (a, wa);
        s[1] = (b, 1.0 - wa);
    }
    s
}

/// The two heaviest classes and a's weight within a + b (128..=255): `[a, b, weight]`.
pub(super) fn top_two(s: &Shares) -> [u8; 3] {
    let (a, wa) = s[0];
    let (b, wb) = s[1];
    if wb <= 0.0 {
        return [a, a, 255];
    }
    [
        a,
        b,
        (255.0 * wa / (wa + wb)).round().clamp(128.0, 255.0) as u8,
    ]
}

/// Area mean of class shares over the same footprint; the heaviest `SHARES` classes are kept.
pub(super) struct ClassMean;

impl Reduce for ClassMean {
    type Row = Vec<Shares>;
    fn reduce(&self, [a, b, c]: [&Vec<Shares>; 3]) -> Vec<Shares> {
        debug_assert!(a.len() == b.len() && c.len() == b.len() && b.len() % 2 == 0);
        (0..b.len() / 2)
            .map(|i| {
                let t = taps(i);
                let (mut sum, mut n) = ([(0u8, 0f32); 9 * SHARES], 0);
                for (r, wy) in [a, b, c].iter().zip(W) {
                    for (&x, wx) in t.iter().zip(W) {
                        for &(class, w) in r[x].iter().filter(|e| e.1 > 0.0) {
                            match sum[..n].iter_mut().find(|e| e.0 == class) {
                                Some(e) => e.1 += wy * wx * w,
                                None => {
                                    sum[n] = (class, wy * wx * w);
                                    n += 1;
                                }
                            }
                        }
                    }
                }
                let sum = &mut sum[..n];
                // Heaviest first, ties to the lowest id.
                sum.sort_by(|p, q| q.1.total_cmp(&p.1).then(p.0.cmp(&q.0)));
                let mut out = [(0, 0.0); SHARES];
                for (o, s) in out.iter_mut().zip(sum.iter()) {
                    *o = *s;
                }
                out
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
/// Widths and row counts must be even down to `last` (power-of-two worlds): an odd width
/// drops the last column, and an odd row count leaves the last row unreduced.
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
    pub fn push(&mut self, mut row: R::Row, sink: &mut Sink<'_, R::Row>) -> Result<()> {
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
    fn class_shares_are_area_means_with_top_two_re_picked() {
        // Coarse 0 (taps 0, 0, 1 per axis): class 1 everywhere except fine column 1, class 2.
        let row = vec![
            shares(1, 1, 255),
            shares(2, 2, 255),
            shares(1, 1, 255),
            shares(1, 1, 255),
        ];
        let out = ClassMean.reduce([&row, &row, &row]);
        // Column weights 0.25 + 0.5 for class 1, 0.25 for class 2.
        assert_eq!(top_two(&out[0]), [1, 2, 191]);
        // Coarse 1 (taps 1, 2, 3): class 2 has 0.25, class 1 0.75.
        assert_eq!(top_two(&out[1]), [1, 2, 191]);
        // A blended texel keeps its blend; one class is a = b, weight 255.
        let mixed = vec![shares(5, 7, 192); 4];
        assert_eq!(
            top_two(&ClassMean.reduce([&mixed, &mixed, &mixed])[0]),
            [5, 7, 192]
        );
        assert_eq!(top_two(&shares(3, 3, 200)), [3, 3, 255]);
    }
    #[test]
    fn sink_sees_rows_in_order_from_first_level() {
        let mut seen = Vec::new();
        let mut p = Pyramid::new(Mean, 2, 4);
        for _ in 0..8 {
            p.push(vec![1.0f32; 8], &mut |l, y, r| {
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
