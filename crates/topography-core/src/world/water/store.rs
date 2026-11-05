//! The work directory: each pass's per-tile results and the global results, so an
//! interrupted run resumes; a small little-endian codec (`Blob`); and `for_tiles`.

use super::geom::Rect;
use crate::acquisition::{AcquisitionError, Result, ensure, io_context};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

pub(crate) struct Enc(pub Vec<u8>);

pub(crate) struct Dec<'a> {
    b: &'a [u8],
    at: usize,
}

impl Dec<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure(self.at + n <= self.b.len(), "work file is truncated")?;
        self.at += n;
        Ok(&self.b[self.at - n..self.at])
    }
}

pub(crate) trait Blob: Sized {
    fn put(&self, e: &mut Enc);
    fn get(d: &mut Dec) -> Result<Self>;
}

macro_rules! blob_num {
    ($t:ty) => {
        impl Blob for $t {
            fn put(&self, e: &mut Enc) {
                e.0.extend_from_slice(&self.to_le_bytes());
            }
            fn get(d: &mut Dec) -> Result<Self> {
                let b = d.take(std::mem::size_of::<$t>())?;
                Ok(<$t>::from_le_bytes(b.try_into().expect("width")))
            }
        }
    };
}
blob_num!(u8);
blob_num!(u32);
blob_num!(u64);
blob_num!(i64);
blob_num!(f32);

impl Blob for bool {
    fn put(&self, e: &mut Enc) {
        u8::from(*self).put(e);
    }
    fn get(d: &mut Dec) -> Result<Self> {
        Ok(u8::get(d)? != 0)
    }
}

impl<T: Blob> Blob for Vec<T> {
    fn put(&self, e: &mut Enc) {
        (self.len() as u64).put(e);
        for v in self {
            v.put(e);
        }
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let n = u64::get(d)? as usize;
        ensure(n <= d.b.len(), "work file length is corrupt")?;
        (0..n).map(|_| T::get(d)).collect()
    }
}

impl<const N: usize> Blob for [u64; N] {
    fn put(&self, e: &mut Enc) {
        for v in self {
            v.put(e);
        }
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let mut out = [0u64; N];
        for v in &mut out {
            *v = u64::get(d)?;
        }
        Ok(out)
    }
}

impl<K: Blob + Ord, V: Blob> Blob for BTreeMap<K, V> {
    fn put(&self, e: &mut Enc) {
        (self.len() as u64).put(e);
        for (k, v) in self {
            k.put(e);
            v.put(e);
        }
    }
    fn get(d: &mut Dec) -> Result<Self> {
        let n = u64::get(d)?;
        (0..n).map(|_| Ok((K::get(d)?, V::get(d)?))).collect()
    }
}

macro_rules! blob_tuple {
    ($($n:ident),+) => {
        impl<$($n: Blob),+> Blob for ($($n,)+) {
            #[allow(non_snake_case)]
            fn put(&self, e: &mut Enc) {
                let ($($n,)+) = self;
                $($n.put(e);)+
            }
            fn get(d: &mut Dec) -> Result<Self> {
                Ok(($($n::get(d)?,)+))
            }
        }
    };
}
blob_tuple!(A, B);
blob_tuple!(A, B, C);
blob_tuple!(A, B, C, D);
blob_tuple!(A, B, C, D, E);

pub(crate) fn encode<T: Blob>(v: &T) -> Vec<u8> {
    let mut e = Enc(Vec::new());
    v.put(&mut e);
    e.0
}

pub(crate) fn decode<T: Blob>(b: &[u8]) -> Result<T> {
    let mut d = Dec { b, at: 0 };
    let v = T::get(&mut d)?;
    ensure(d.at == b.len(), "work file has trailing bytes")?;
    Ok(v)
}

pub(crate) struct Work {
    dir: PathBuf,
}

impl Work {
    /// Opens (creating) a work directory for one set of inputs. `stamp` (input digests, tile
    /// size, tool version) must equal what an earlier run wrote, so a resume never mixes runs.
    pub fn open(dir: &Path, stamp: &Value) -> Result<Self> {
        io_context(format!("create {}", dir.display()), fs::create_dir_all(dir))?;
        let path = dir.join("stamp.json");
        if path.exists() {
            let old: Value = serde_json::from_slice(&io_context("read stamp", fs::read(&path))?)?;
            ensure(
                &old == stamp,
                format!(
                    "{} belongs to other inputs or settings; remove it to start again",
                    dir.display()
                ),
            )?;
        } else {
            io_context(
                "write stamp",
                fs::write(&path, serde_json::to_vec_pretty(stamp)?),
            )?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    /// A result read back when present, else computed and written (atomically).
    pub fn cached<T: Blob>(&self, name: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let path = self.dir.join(format!("{name}.bin"));
        if path.exists() {
            return decode(&io_context(
                format!("read {}", path.display()),
                fs::read(&path),
            )?);
        }
        let v = f()?;
        if let Some(parent) = path.parent() {
            io_context("create work dir", fs::create_dir_all(parent))?;
        }
        let tmp = path.with_extension("partial");
        io_context("write work file", fs::write(&tmp, encode(&v)))?;
        io_context("publish work file", fs::rename(&tmp, &path))?;
        Ok(v)
    }
    pub fn is_done(&self, name: &str) -> bool {
        self.dir.join(format!("{name}.done")).exists()
    }
    pub fn mark_done(&self, name: &str) -> Result<()> {
        let path = self.dir.join(format!("{name}.done"));
        if let Some(parent) = path.parent() {
            io_context("create work dir", fs::create_dir_all(parent))?;
        }
        io_context("mark done", fs::write(path, b""))
    }
}

/// `Work::cached` when there is a work directory; in-memory runs compute every time.
pub(crate) fn cached<T: Blob>(
    work: Option<&Work>,
    name: &str,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    match work {
        Some(w) => w.cached(name, f),
        None => f(),
    }
}

/// Runs `f` on every tile, `threads` at a time; results in tile order. Progress goes to
/// stderr as `water: <label> <done>/<tiles>` lines.
pub(crate) fn for_tiles<T: Send>(
    tiles: &[Rect],
    threads: usize,
    label: &str,
    f: impl Fn(usize, Rect) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<Result<T>>>> = tiles.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= tiles.len() {
                        break;
                    }
                    let r = f(i, tiles[i]);
                    *slots[i].lock().expect("slot") = Some(r);
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if tiles.len() > 16 && d % 16 == 0 {
                        eprintln!(
                            "water: {label} {d}/{} ({:.0} s)",
                            tiles.len(),
                            started.elapsed().as_secs_f64()
                        );
                    }
                }
            });
        }
    });
    eprintln!(
        "water: {label} done in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .expect("slot")
                .unwrap_or_else(|| Err(AcquisitionError::Invalid("a tile did not run".into())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Mixed = (Vec<u8>, BTreeMap<u64, (f32, Vec<u32>)>, i64, [u64; 4], bool);

    #[test]
    fn blobs_round_trip() {
        let mut m = BTreeMap::new();
        m.insert(3u64, (1.5f32, vec![1u32, 2]));
        let v = (vec![7u8, 9], m, -4i64, [1u64, 2, 3, 4], true);
        let back: Mixed = decode(&encode(&v)).expect("decode");
        assert_eq!(back, v);
        assert!(decode::<u64>(&[1, 2]).is_err());
    }

    #[test]
    fn work_caches_and_refuses_other_inputs() {
        let dir = tempfile::tempdir().expect("temp");
        let stamp = serde_json::json!({"input": "a"});
        let calls = AtomicUsize::new(0);
        let work = Work::open(dir.path(), &stamp).expect("open");
        let f = || -> Result<u64> {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(42)
        };
        assert_eq!(work.cached("m1/a/0000", f).expect("first"), 42);
        assert_eq!(work.cached("m1/a/0000", f).expect("second"), 42);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(Work::open(dir.path(), &serde_json::json!({"input": "b"})).is_err());
    }

    #[test]
    fn for_tiles_keeps_tile_order() {
        let tiles: Vec<Rect> = (0..9)
            .map(|i| Rect {
                x0: i,
                y0: 0,
                w: 1,
                h: 1,
            })
            .collect();
        let got = for_tiles(&tiles, 4, "test", |i, r| Ok(i as u64 * 10 + r.x0)).expect("run");
        assert_eq!(got, (0..9).map(|i| i * 11).collect::<Vec<u64>>());
    }
}
