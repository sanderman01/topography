//! Grid directory IO: `grid.json`, little-endian rasters (rows north to south) and an
//! output stage that is published by one rename.

use crate::acquisition::{AcquisitionError, Result, ensure, io_context, read_small};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

/// Grid shape and the input manifest, passed through to the output.
pub(super) struct Grid {
    pub w: usize,
    pub h: usize,
    pub spacing_m: f64,
    /// North-west corner in the layout plane (x east, y north): `origin` plus `offset_m`
    /// ([east, south] of a band or window inside the world).
    pub nw_m: [f64; 2],
    /// Rows this grid delivers (`core_rows` [start, end)); the rest is halo.
    pub core_rows: [usize; 2],
    pub manifest: Map<String, Value>,
}

/// `grid.json` fields: size, spacing, origin, offset and core rows.
pub(super) fn shape(manifest: &Map<String, Value>) -> Result<Grid> {
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
    let core_rows: [usize; 2] = match manifest.get("core_rows") {
        None | Some(Value::Null) => [0, h],
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| {
            AcquisitionError::Invalid("grid.json core_rows must be [start, end]".into())
        })?,
    };
    ensure(
        core_rows[0] < core_rows[1] && core_rows[1] <= h,
        "grid.json core_rows must be [start, end] within the grid",
    )?;
    Ok(Grid {
        w,
        h,
        spacing_m,
        nw_m: [origin[0] + offset[0], origin[1] - offset[1]],
        core_rows,
        manifest: manifest.clone(),
    })
}

impl Grid {
    pub fn cells(&self) -> usize {
        self.w * self.h
    }
    pub fn cell_km2(&self) -> f64 {
        self.spacing_m * self.spacing_m / 1e6
    }
}

/// Input rasters with their SHA-256 digests by file name.
pub(super) struct Input {
    pub grid: Grid,
    pub elevation: Vec<f32>,
    pub classes: Option<Vec<u8>>,
    pub digests: Map<String, Value>,
}

pub(super) fn read(dir: &Path) -> Result<Input> {
    let manifest_bytes = read_small(&dir.join("grid.json"))?;
    let manifest: Map<String, Value> = serde_json::from_slice(&manifest_bytes)?;
    let grid = shape(&manifest)?;
    let (w, h, spacing_m) = (grid.w, grid.h, grid.spacing_m);
    ensure(w >= 3 && h >= 3, "grid must be at least 3 x 3 cells")?;
    // Cell indices are u32.
    let n = w
        .checked_mul(h)
        .filter(|&n| n < u32::MAX as usize)
        .ok_or_else(|| AcquisitionError::Invalid("grid has too many cells".into()))?;
    let mut digests = Map::new();
    digests.insert(
        "grid.json".into(),
        hex(&Sha256::digest(&manifest_bytes)).into(),
    );

    let (bytes, sha) = read_raw(&dir.join("elevation.f32"), n * 4)?;
    digests.insert("elevation.f32".into(), sha.into());
    let elevation: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    drop(bytes);
    ensure(
        elevation.iter().all(|v| v.is_finite()),
        "elevation.f32 holds non-finite values",
    )?;

    let path = dir.join("classes.u8");
    let classes = if path.exists() {
        let cell_m = grid
            .manifest
            .get("classes_cell_m")
            .and_then(Value::as_f64)
            .unwrap_or(spacing_m);
        let k = cell_m / spacing_m;
        ensure(
            k >= 1.0 && k.fract() == 0.0,
            "classes_cell_m must be a whole multiple of spacing_m",
        )?;
        let k = k as usize;
        let (cw, ch) = (w.div_ceil(k), h.div_ceil(k));
        let (coarse, sha) = read_raw(&path, cw * ch)?;
        digests.insert("classes.u8".into(), sha.into());
        Some(if k == 1 {
            coarse
        } else {
            (0..n)
                .map(|i| coarse[(i / w / k) * cw + (i % w) / k])
                .collect()
        })
    } else {
        None
    };
    Ok(Input {
        grid,
        elevation,
        classes,
        digests,
    })
}

/// A whole file of exactly `len` bytes and its SHA-256.
fn read_raw(path: &Path, len: usize) -> Result<(Vec<u8>, String)> {
    let mut file = io_context(format!("open {}", path.display()), fs::File::open(path))?;
    let size = io_context("inspect input", file.metadata())?.len();
    ensure(
        size == len as u64,
        format!("{} has {size} bytes, expected {len}", path.display()),
    )?;
    let mut bytes = vec![0u8; len];
    let mut hasher = Sha256::new();
    for chunk in bytes.chunks_mut(1 << 20) {
        io_context(format!("read {}", path.display()), file.read_exact(chunk))?;
        hasher.update(&chunk[..]);
    }
    Ok((bytes, hex(&hasher.finalize())))
}

/// Elevation of an input grid, streamed row by row (for comparisons after carving).
pub(super) fn for_each_input_row(
    dir: &Path,
    w: usize,
    mut f: impl FnMut(usize, &[f32]),
) -> Result<()> {
    let path = dir.join("elevation.f32");
    let mut file = io_context(format!("open {}", path.display()), fs::File::open(&path))?;
    let mut bytes = vec![0u8; w * 4];
    let mut row = vec![0f32; w];
    let mut y = 0;
    loop {
        match file.read_exact(&mut bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return io_context("read elevation", Err(e)),
        }
        for (v, b) in row.iter_mut().zip(bytes.chunks_exact(4)) {
            *v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
        f(y, &row);
        y += 1;
    }
}

/// Rows `y0..y1`, columns `x0..x1` of a `w` x `h` raster of `B`-byte little-endian cells.
pub(super) fn read_window<const B: usize, T>(
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
pub(super) struct Stage {
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

    /// A raster from `n` values, each encoded as little-endian bytes by `f`.
    pub fn raster<const B: usize>(
        &mut self,
        name: &str,
        n: usize,
        f: impl Fn(usize) -> [u8; B],
    ) -> Result<()> {
        let path = self.dir.path().join(name);
        let file = io_context(format!("create {name}"), fs::File::create(&path))?;
        let mut writer = BufWriter::with_capacity(1 << 20, file);
        let mut hasher = Sha256::new();
        let mut buffer = Vec::with_capacity(1 << 16);
        for start in (0..n).step_by(1 << 14) {
            buffer.clear();
            for i in start..(start + (1 << 14)).min(n) {
                buffer.extend_from_slice(&f(i));
            }
            hasher.update(&buffer);
            io_context(format!("write {name}"), writer.write_all(&buffer))?;
        }
        let file = io_context(
            format!("flush {name}"),
            writer.into_inner().map_err(|e| e.into_error()),
        )?;
        io_context(format!("sync {name}"), file.sync_all())?;
        self.digests
            .insert(name.into(), hex(&hasher.finalize()).into());
        Ok(())
    }

    pub fn json(&mut self, name: &str, value: &Value) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(value)?;
        io_context(
            format!("write {name}"),
            fs::write(self.dir.path().join(name), &bytes),
        )?;
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
