//! World-sized rasters, in memory (tests, windows) or in little-endian files (the world):
//! windowed reads and positioned writes, so tiles never need the whole raster.

use super::geom::{Rect, World};
use super::store::partial_path;
use crate::acquisition::{Result, ensure, io_context};
use std::{
    fs,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

/// A raster cell type stored little-endian.
pub(crate) trait Cell: Copy + Send + Sync + PartialEq + 'static {
    const B: usize;
    fn get(b: &[u8]) -> Self;
    fn put(self, out: &mut Vec<u8>);
}

macro_rules! cell {
    ($t:ty) => {
        impl Cell for $t {
            const B: usize = std::mem::size_of::<$t>();
            fn get(b: &[u8]) -> Self {
                <$t>::from_le_bytes(b.try_into().expect("cell width"))
            }
            fn put(self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
        }
    };
}
cell!(f32);
cell!(u8);
cell!(u32);
cell!(u64);

#[derive(Clone)]
pub(crate) enum Layer<T: Cell> {
    Memory {
        world: World,
        values: Arc<RwLock<Vec<T>>>,
    },
    File {
        world: World,
        path: PathBuf,
    },
}

impl<T: Cell> Layer<T> {
    pub fn memory(world: World, values: Vec<T>) -> Self {
        assert_eq!(values.len() as u64, world.w * world.h);
        Layer::Memory {
            world,
            values: Arc::new(RwLock::new(values)),
        }
    }
    pub fn filled(world: World, fill: T) -> Self {
        Self::memory(world, vec![fill; (world.w * world.h) as usize])
    }
    /// An existing file of exactly the world's size.
    pub fn open(world: World, path: &Path) -> Result<Self> {
        let size = io_context(format!("inspect {}", path.display()), fs::metadata(path))?.len();
        ensure(
            size == world.w * world.h * T::B as u64,
            format!(
                "{} has {size} bytes, expected {}",
                path.display(),
                world.w * world.h * T::B as u64
            ),
        )?;
        Ok(Layer::File {
            world,
            path: path.to_path_buf(),
        })
    }
    /// The file at `path`, created full of `fill` when missing (a resumed run keeps it).
    pub fn open_or_create(world: World, path: &Path, fill: T) -> Result<Self> {
        if path.exists() {
            return Self::open(world, path);
        }
        let tmp = partial_path(path);
        let file = io_context(format!("create {}", tmp.display()), fs::File::create(&tmp))?;
        let mut row = Vec::with_capacity(world.w as usize * T::B);
        for _ in 0..world.w {
            fill.put(&mut row);
        }
        for y in 0..world.h {
            io_context("fill layer", file.write_all_at(&row, y * row.len() as u64))?;
        }
        io_context("sync layer", file.sync_all())?;
        io_context("publish layer", fs::rename(&tmp, path))?;
        Self::open(world, path)
    }
    pub fn world(&self) -> World {
        match self {
            Layer::Memory { world, .. } | Layer::File { world, .. } => *world,
        }
    }
    pub fn read(&self, r: Rect) -> Result<Vec<T>> {
        let world = self.world();
        ensure(
            r.x0 + r.w as u64 <= world.w && r.y0 + r.h as u64 <= world.h,
            "raster window out of range",
        )?;
        let mut out = Vec::with_capacity(r.cells());
        match self {
            Layer::Memory { values, .. } => {
                let v = values.read().expect("layer lock");
                for y in r.y0..r.y0 + r.h as u64 {
                    let at = (y * world.w + r.x0) as usize;
                    out.extend_from_slice(&v[at..at + r.w]);
                }
            }
            Layer::File { path, .. } => {
                let file = io_context(format!("open {}", path.display()), fs::File::open(path))?;
                let mut bytes = vec![0u8; r.w * T::B];
                for y in r.y0..r.y0 + r.h as u64 {
                    let at = (y * world.w + r.x0) * T::B as u64;
                    io_context(
                        format!("read {}", path.display()),
                        file.read_exact_at(&mut bytes, at),
                    )?;
                    out.extend(bytes.chunks_exact(T::B).map(T::get));
                }
            }
        }
        Ok(out)
    }
    /// Writes `values` into window `r`. Parallel tiles share the layer without a lock, so
    /// callers write only their tile's core, never its halo: cores never overlap.
    pub fn write(&self, r: Rect, values: &[T]) -> Result<()> {
        let world = self.world();
        ensure(
            r.x0 + r.w as u64 <= world.w && r.y0 + r.h as u64 <= world.h,
            "raster window out of range",
        )?;
        ensure(values.len() == r.cells(), "raster write size")?;
        match self {
            Layer::Memory { values: v, .. } => {
                let mut v = v.write().expect("layer lock");
                for (row, y) in (r.y0..r.y0 + r.h as u64).enumerate() {
                    let at = (y * world.w + r.x0) as usize;
                    v[at..at + r.w].copy_from_slice(&values[row * r.w..(row + 1) * r.w]);
                }
            }
            Layer::File { path, .. } => {
                let file = io_context(
                    format!("open {}", path.display()),
                    fs::OpenOptions::new().write(true).open(path),
                )?;
                let mut bytes = Vec::with_capacity(r.w * T::B);
                for (row, y) in (r.y0..r.y0 + r.h as u64).enumerate() {
                    bytes.clear();
                    for &c in &values[row * r.w..(row + 1) * r.w] {
                        c.put(&mut bytes);
                    }
                    let at = (y * world.w + r.x0) * T::B as u64;
                    io_context(
                        format!("write {}", path.display()),
                        file.write_all_at(&bytes, at),
                    )?;
                }
            }
        }
        Ok(())
    }
    /// The whole raster (tests and small windows).
    pub fn to_vec(&self) -> Result<Vec<T>> {
        self.read(Rect::whole(self.world()))
    }
}

/// Classes on a coarser grid: `k` grid cells per class cell side.
#[derive(Clone)]
pub(crate) struct Classes {
    pub layer: Layer<u8>,
    pub k: u64,
}

impl Classes {
    /// Classes for `world`; the class grid must cover it.
    pub fn new(layer: Layer<u8>, k: u64, world: World) -> Result<Self> {
        let cw = layer.world();
        ensure(
            k > 0 && cw.w * k >= world.w && cw.h * k >= world.h,
            format!(
                "class grid {}x{} at {k} cells per class does not cover the {}x{} world",
                cw.w, cw.h, world.w, world.h
            ),
        )?;
        Ok(Self { layer, k })
    }

    pub fn read(&self, r: Rect) -> Result<Vec<u8>> {
        let cw = self.layer.world();
        ensure(r.cells() > 0, "empty class window")?;
        let (cx0, cy0) = (r.x0 / self.k, r.y0 / self.k);
        let cx1 = (r.x0 + r.w as u64 - 1) / self.k + 1;
        let cy1 = (r.y0 + r.h as u64 - 1) / self.k + 1;
        ensure(
            cx1 <= cw.w && cy1 <= cw.h,
            "class window lies beyond the class grid",
        )?;
        let c = Rect {
            x0: cx0,
            y0: cy0,
            w: (cx1 - cx0) as usize,
            h: (cy1 - cy0) as usize,
        };
        let coarse = self.layer.read(c)?;
        Ok((0..r.cells())
            .map(|k| {
                let (x, y) = r.world_xy(k);
                let (cx, cy) = (x / self.k, y / self.k);
                coarse[(cy - cy0) as usize * c.w + (cx - cx0) as usize]
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_layer_reads_and_writes_windows() {
        let dir = tempfile::tempdir().expect("temp");
        let world = World { w: 5, h: 4 };
        let layer = Layer::<f32>::open_or_create(world, &dir.path().join("a.f32"), f32::NAN)
            .expect("create");
        let r = Rect {
            x0: 1,
            y0: 1,
            w: 3,
            h: 2,
        };
        layer
            .write(r, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
            .expect("write");
        let back = layer
            .read(Rect {
                x0: 0,
                y0: 1,
                w: 5,
                h: 1,
            })
            .expect("read");
        assert!(back[0].is_nan() && back[4].is_nan());
        assert_eq!(&back[1..4], &[1.0, 2.0, 3.0]);
        let again =
            Layer::<f32>::open_or_create(world, &dir.path().join("a.f32"), 0.0).expect("reopen");
        assert_eq!(
            again.read(r).expect("read"),
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            "kept"
        );
    }

    #[test]
    fn classes_expand_their_coarse_cells() {
        let coarse = Layer::memory(World { w: 2, h: 2 }, vec![1u8, 2, 3, 4]);
        let classes = Classes {
            layer: coarse,
            k: 4,
        };
        let got = classes
            .read(Rect {
                x0: 3,
                y0: 3,
                w: 2,
                h: 2,
            })
            .expect("read");
        assert_eq!(got, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_write_outside_the_world_is_refused() {
        let dir = tempfile::tempdir().expect("temp");
        let world = World { w: 3, h: 3 };
        let file = Layer::<u8>::open_or_create(world, &dir.path().join("a.u8"), 0).expect("create");
        let memory = Layer::<u8>::filled(world, 0);
        for layer in [file, memory] {
            let r = Rect {
                x0: 2,
                y0: 0,
                w: 2,
                h: 1,
            };
            assert!(layer.write(r, &[1, 1]).is_err());
            assert_eq!(layer.to_vec().expect("read"), vec![0; 9], "nothing written");
        }
    }

    #[test]
    fn classes_must_cover_the_world_and_the_window() {
        let world = World { w: 9, h: 8 };
        let coarse = || Layer::memory(World { w: 2, h: 2 }, vec![1u8, 2, 3, 4]);
        assert!(
            Classes::new(coarse(), 4, world).is_err(),
            "8 < 9 cells wide"
        );
        assert!(Classes::new(coarse(), 0, world).is_err());
        let classes = Classes::new(coarse(), 5, world).expect("covers");
        assert_eq!(classes.k, 5);
        let small = Classes {
            layer: coarse(),
            k: 4,
        };
        let r = Rect {
            x0: 6,
            y0: 0,
            w: 3,
            h: 1,
        };
        assert!(small.read(r).is_err(), "x 8 lies beyond the class grid");
    }
}
