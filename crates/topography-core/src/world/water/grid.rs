//! Grid directory IO: `grid.json`, little-endian rasters (rows north to south) and an
//! output stage that is published by one rename.

use crate::acquisition::{AcquisitionError, Result, ensure, io_context};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Grid shape from `grid.json`.
pub(crate) struct Grid {
    pub w: usize,
    pub h: usize,
    pub spacing_m: f64,
    /// North-west corner in the layout plane (x east, y north): `origin` plus `offset_m`
    /// ([east, south] of a window inside the world).
    pub nw_m: [f64; 2],
}

/// `grid.json` fields: size, spacing, origin and offset.
pub(crate) fn shape(manifest: &Map<String, Value>) -> Result<Grid> {
    let size: [usize; 2] =
        serde_json::from_value(manifest.get("size").cloned().unwrap_or_default())
            .map_err(|_| AcquisitionError::Invalid("grid.json size must be [w, h]".into()))?;
    let spacing_m = manifest
        .get("spacing_m")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    ensure(
        spacing_m.is_finite() && spacing_m > 0.0,
        "grid.json spacing_m must be a positive number",
    )?;
    let [w, h] = size;
    let pair = |key: &str, default: [f64; 2]| -> Result<[f64; 2]> {
        match manifest.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => serde_json::from_value::<[f64; 2]>(v.clone())
                .ok()
                .filter(|p| p.iter().all(|c| c.is_finite()))
                .ok_or_else(|| {
                    AcquisitionError::Invalid(format!("grid.json {key} must be [x, y]"))
                }),
        }
    };
    let origin = pair("origin", [0.0, spacing_m * h as f64])?;
    let offset = pair("offset_m", [0.0, 0.0])?;
    Ok(Grid {
        w,
        h,
        spacing_m,
        nw_m: [origin[0] + offset[0], origin[1] - offset[1]],
    })
}

/// A body list (`bodies.json`): a 1 m world's thousands of basins pass the 1 MiB metadata limit.
pub(crate) fn read_bodies(path: &Path) -> Result<Vec<u8>> {
    const LIMIT: u64 = 256 << 20;
    let file = io_context(format!("open {}", path.display()), fs::File::open(path))?;
    let mut bytes = Vec::new();
    io_context(
        format!("read {}", path.display()),
        file.take(LIMIT + 1).read_to_end(&mut bytes),
    )?;
    ensure(
        bytes.len() as u64 <= LIMIT,
        format!("{} exceeds 256 MiB", path.display()),
    )?;
    Ok(bytes)
}

/// Rows `y0..y1`, columns `x0..x1` of a `w` x `h` raster of `B`-byte little-endian cells.
pub(crate) fn read_window<const B: usize, T>(
    path: &Path,
    [w, h]: [usize; 2],
    [x0, x1]: [usize; 2],
    [y0, y1]: [usize; 2],
    decode: impl Fn([u8; B]) -> T,
) -> Result<Vec<T>> {
    use std::io::{Seek, SeekFrom};
    let mut file = io_context(format!("open {}", path.display()), fs::File::open(path))?;
    let size = io_context("inspect input", file.metadata())?.len();
    ensure(
        size == (w * h * B) as u64,
        format!(
            "{} has {size} bytes, expected {}",
            path.display(),
            w * h * B
        ),
    )?;
    ensure(
        x0 < x1 && x1 <= w && y0 < y1 && y1 <= h,
        "raster window out of range",
    )?;
    let mut row = vec![0u8; (x1 - x0) * B];
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        let at = ((y * w + x0) * B) as u64;
        io_context("seek", file.seek(SeekFrom::Start(at)))?;
        io_context(
            format!("read {}", path.display()),
            file.read_exact(&mut row),
        )?;
        out.extend(
            row.chunks_exact(B)
                .map(|c| decode(c.try_into().unwrap_or([0; B]))),
        );
    }
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Output files written into a temporary sibling of `out`, then renamed into place.
pub(crate) struct Stage {
    dir: tempfile::TempDir,
    out: PathBuf,
    pub digests: Map<String, Value>,
}

impl Stage {
    pub fn new(out: &Path) -> Result<Self> {
        ensure(!out.exists(), format!("{} already exists", out.display()))?;
        let parent = out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        io_context("create output parent", fs::create_dir_all(parent))?;
        let dir = io_context("stage output", tempfile::tempdir_in(parent))?;
        Ok(Self {
            dir,
            out: out.to_path_buf(),
            digests: Map::new(),
        })
    }

    /// A finished file (e.g. a 17 GB raster from a work directory) copied into the stage as a
    /// reflink where the file system has them (btrfs: no extra space), a plain copy elsewhere,
    /// never a hard link: work rasters are written in place, and a rerun must not change a
    /// published output behind its digest (N17). Then digested.
    pub fn copy(&mut self, name: &str, src: &Path) -> Result<()> {
        let dst = self.dir.path().join(name);
        let done = std::process::Command::new("cp")
            .arg("--reflink=auto")
            .arg(src)
            .arg(&dst)
            .status();
        ensure(
            io_context(format!("copy {}", src.display()), done)?.success(),
            format!("copy {} failed", src.display()),
        )?;
        self.digest(name)
    }

    /// Digest a staged file (and sync it).
    fn digest(&mut self, name: &str) -> Result<()> {
        let path = self.dir.path().join(name);
        let mut file = io_context(format!("open {name}"), fs::File::open(path))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 22];
        loop {
            let n = io_context(format!("read {name}"), file.read(&mut buf))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        // On disk before `publish` renames the stage into place.
        io_context(format!("sync {name}"), file.sync_all())?;
        self.digests
            .insert(name.into(), hex(&hasher.finalize()).into());
        Ok(())
    }

    pub fn json(&mut self, name: &str, value: &Value) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(value)?;
        let mut file = io_context(
            format!("create {name}"),
            fs::File::create(self.dir.path().join(name)),
        )?;
        io_context(format!("write {name}"), file.write_all(&bytes))?;
        io_context(format!("sync {name}"), file.sync_all())?;
        self.digests
            .insert(name.into(), hex(&Sha256::digest(&bytes)).into());
        Ok(())
    }

    pub fn publish(self) -> Result<PathBuf> {
        let path = self.dir.keep();
        io_context(
            format!("publish {}", self.out.display()),
            fs::rename(&path, &self.out),
        )?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staged_copy_is_digested_and_never_shares_the_source() {
        let dir = tempfile::tempdir().expect("temp dir");
        let src = dir.path().join("work.f32");
        fs::write(&src, b"abcd").expect("source");
        let mut stage = Stage::new(&dir.path().join("out")).expect("stage");
        stage.copy("elevation.f32", &src).expect("copy");
        assert_eq!(
            stage.digests["elevation.f32"],
            hex(&Sha256::digest(b"abcd"))
        );
        // A rerun writing the work file in place leaves the published copy alone (N17).
        fs::OpenOptions::new()
            .write(true)
            .open(&src)
            .and_then(|mut f| f.write_all(b"wxyz"))
            .expect("rewrite");
        let out = stage.publish().expect("publish");
        assert_eq!(fs::read(out.join("elevation.f32")).expect("read"), b"abcd");
    }

    #[test]
    fn a_body_list_over_1_mib_is_read() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("bodies.json");
        let big = format!("{{\"bodies\": [], \"pad\": \"{}\"}}", "x".repeat(3 << 20));
        fs::write(&path, &big).expect("write");
        assert_eq!(read_bodies(&path).expect("read").len(), big.len());
    }
}
