//! World and tile geometry. Cells are indexed in the world (u64, rows north to south); a tile
//! is a block of them, a window a block grown by a halo and clipped to the world.

/// D8 offsets (row, column); direction byte = index + 1: 1 N, 2 S, 3 W, 4 E, 5 NW, 6 NE, 7 SW,
/// 8 SE (as v2's `flow.u8`).
pub(crate) const OFFSETS: [(i64, i64); 8] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (1, 1),
];

/// Step length in cells for a direction byte.
pub(crate) fn step_len(d: u8) -> f64 {
    if d > 4 { std::f64::consts::SQRT_2 } else { 1.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct World {
    pub w: u64,
    pub h: u64,
}

impl World {
    pub fn index(self, x: u64, y: u64) -> u64 {
        y * self.w + x
    }
    pub fn xy(self, i: u64) -> (u64, u64) {
        (i % self.w, i / self.w)
    }
    pub fn is_edge(self, x: u64, y: u64) -> bool {
        x == 0 || y == 0 || x + 1 == self.w || y + 1 == self.h
    }
    /// Neighbour `k` (an `OFFSETS` index) of (x, y), if inside the world.
    pub fn neighbour(self, x: u64, y: u64, k: usize) -> Option<(u64, u64)> {
        let (dy, dx) = OFFSETS[k];
        let nx = x.checked_add_signed(dx)?;
        let ny = y.checked_add_signed(dy)?;
        (nx < self.w && ny < self.h).then_some((nx, ny))
    }
}

/// A block of cells in world coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x0: u64,
    pub y0: u64,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn whole(world: World) -> Self {
        Rect {
            x0: 0,
            y0: 0,
            w: world.w as usize,
            h: world.h as usize,
        }
    }
    pub fn cells(self) -> usize {
        self.w * self.h
    }
    pub fn contains(self, x: u64, y: u64) -> bool {
        x >= self.x0 && y >= self.y0 && x < self.x0 + self.w as u64 && y < self.y0 + self.h as u64
    }
    /// Index of world cell (x, y) in this block, if inside.
    pub fn local(self, x: u64, y: u64) -> Option<usize> {
        self.contains(x, y)
            .then(|| (y - self.y0) as usize * self.w + (x - self.x0) as usize)
    }
    pub fn world_xy(self, k: usize) -> (u64, u64) {
        (self.x0 + (k % self.w) as u64, self.y0 + (k / self.w) as u64)
    }
    /// This block grown by `halo` cells on every side, clipped to the world.
    pub fn grow(self, halo: u64, world: World) -> Rect {
        let x0 = self.x0.saturating_sub(halo);
        let y0 = self.y0.saturating_sub(halo);
        let x1 = (self.x0 + self.w as u64 + halo).min(world.w);
        let y1 = (self.y0 + self.h as u64 + halo).min(world.h);
        Rect {
            x0,
            y0,
            w: (x1 - x0) as usize,
            h: (y1 - y0) as usize,
        }
    }
    /// On this block's outer ring of cells.
    pub fn on_ring(self, x: u64, y: u64) -> bool {
        self.contains(x, y)
            && (x == self.x0
                || y == self.y0
                || x + 1 == self.x0 + self.w as u64
                || y + 1 == self.y0 + self.h as u64)
    }
}

/// Tiles of `size` cells covering the world row by row; the last ones may be smaller.
pub(crate) fn tiles(world: World, size: u64) -> Vec<Rect> {
    assert!(size > 0, "tile size must be positive");
    let mut out = Vec::new();
    for y0 in (0..world.h).step_by(size as usize) {
        for x0 in (0..world.w).step_by(size as usize) {
            out.push(Rect {
                x0,
                y0,
                w: size.min(world.w - x0) as usize,
                h: size.min(world.h - y0) as usize,
            });
        }
    }
    out
}

/// Index (in `tiles` order) of the tile holding (x, y).
pub(crate) fn tile_of(world: World, size: u64, x: u64, y: u64) -> usize {
    assert!(size > 0, "tile size must be positive");
    (y / size * world.w.div_ceil(size) + x / size) as usize
}

/// Union-find over 0..n whose roots are the smallest member (deterministic).
pub(crate) struct UnionFind(Vec<u32>);

impl UnionFind {
    pub fn new(n: usize) -> Self {
        assert!(
            n <= u32::MAX as usize,
            "union-find holds at most u32::MAX members"
        );
        Self((0..n as u32).collect())
    }
    pub fn find(&mut self, mut a: u32) -> u32 {
        while self.0[a as usize] != a {
            let up = self.0[self.0[a as usize] as usize];
            self.0[a as usize] = up;
            a = up;
        }
        a
    }
    /// Joins the sets of `a` and `b`; returns the new root (the smaller one).
    pub fn union(&mut self, a: u32, b: u32) -> u32 {
        let (ra, rb) = (self.find(a), self.find(b));
        let (lo, hi) = (ra.min(rb), ra.max(rb));
        self.0[hi as usize] = lo;
        lo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_the_world_once() {
        let world = World { w: 10, h: 7 };
        let t = tiles(world, 4);
        assert_eq!(t.len(), 3 * 2);
        assert_eq!(t.iter().map(|r| r.cells()).sum::<usize>(), 70);
        assert_eq!(
            t[2],
            Rect {
                x0: 8,
                y0: 0,
                w: 2,
                h: 4
            }
        );
        assert_eq!(tile_of(world, 4, 9, 6), 5);
    }

    #[test]
    fn a_grown_rect_is_clipped_and_maps_back() {
        let world = World { w: 10, h: 10 };
        let core = Rect {
            x0: 0,
            y0: 4,
            w: 4,
            h: 4,
        };
        let win = core.grow(2, world);
        assert_eq!(
            win,
            Rect {
                x0: 0,
                y0: 2,
                w: 6,
                h: 8
            }
        );
        assert_eq!(win.local(1, 3), Some(7));
        assert_eq!(win.world_xy(7), (1, 3));
        assert!(core.on_ring(0, 5) && !core.on_ring(1, 5));
    }

    #[test]
    fn union_find_keeps_the_smaller_root() {
        let mut u = UnionFind::new(5);
        u.union(3, 1);
        u.union(4, 3);
        assert_eq!(u.find(4), 1);
        assert_eq!(u.find(0), 0);
    }

    #[test]
    #[should_panic(expected = "tile size")]
    fn a_zero_tile_size_is_refused() {
        tiles(World { w: 4, h: 4 }, 0);
    }

    #[test]
    #[should_panic(expected = "tile size")]
    fn tile_of_refuses_a_zero_tile_size() {
        tile_of(World { w: 4, h: 4 }, 0, 1, 1);
    }
}
