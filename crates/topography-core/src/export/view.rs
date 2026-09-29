//! Loopback 3D viewer for a heightmap export; reads only the exported tiles.
use crate::acquisition::{AcquisitionError, Result, ensure, io_context};
use crate::review::gallery_server::{request, respond};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
};

const PAGE: &str = include_str!("view.html");
/// Largest region side served at once (about 1 km at 1 m plus a shared edge).
const MAX_SIDE: usize = 2049;

struct Export {
    root: PathBuf,
    manifest: Value,
    intervals: usize,
    h_min: f64,
    step: f64,
    masks: Vec<String>,
    /// Samples per side for each level.
    levels: Vec<usize>,
}

impl Export {
    fn open(root: &Path) -> Result<Self> {
        let text = io_context(
            "read export manifest",
            fs::read_to_string(root.join("manifest.json")),
        )?;
        let manifest: Value = serde_json::from_str(&text)?;
        ensure(
            manifest["kind"] == "heightmap_export",
            "not a heightmap export manifest",
        )?;
        let number = |v: &Value, what: &str| {
            v.as_f64()
                .ok_or_else(|| AcquisitionError::Invalid(format!("export manifest lacks {what}")))
        };
        let intervals = number(&manifest["intervals"], "intervals")? as usize;
        let h_min = number(&manifest["encoding"]["h_min_m"], "h_min_m")?;
        let step = number(&manifest["encoding"]["step_m"], "step_m")?;
        let masks = manifest["masks"]["layers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let levels = manifest["levels"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l["samples_per_side"].as_u64().map(|n| n as usize))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        ensure(intervals > 0 && !levels.is_empty(), "export has no levels")?;
        Ok(Self {
            root: root.into(),
            manifest,
            intervals,
            h_min,
            step,
            masks,
            levels,
        })
    }

    /// Heights (f32 m) and masks for samples [x, x+w) × [y, y+h) of one level.
    fn region(&self, level: usize, x: usize, y: usize, side: usize) -> Result<Vec<u8>> {
        let samples = *self
            .levels
            .get(level)
            .ok_or_else(|| AcquisitionError::Invalid("unknown level".into()))?;
        ensure(side > 0 && side <= MAX_SIDE, "region too large")?;
        ensure(x < samples && y < samples, "region outside the export")?;
        let (w, h) = (side.min(samples - x), side.min(samples - y));
        let mut heights = vec![0f32; w * h];
        let mut masks = vec![vec![0u8; w * h]; self.masks.len()];
        let last = samples.div_ceil(self.intervals) - 1;
        let (t0x, t1x) = (
            (x / self.intervals).min(last),
            ((x + w - 1) / self.intervals).min(last),
        );
        let (t0y, t1y) = (
            (y / self.intervals).min(last),
            ((y + h - 1) / self.intervals).min(last),
        );
        for ty in t0y..=t1y {
            for tx in t0x..=t1x {
                let name = format!("L{level}/{tx}_{ty}.png");
                let layers = std::iter::once("height")
                    .chain(self.masks.iter().map(String::as_str))
                    .enumerate();
                for (index, layer) in layers {
                    let (tw, th, pixels) = read_png(&self.root.join(layer).join(&name))?;
                    let (ox, oy) = (tx * self.intervals, ty * self.intervals);
                    // A sample belongs to the tile its index falls in; shared edges repeat.
                    let rows = oy.max(y)..(oy + th).min(y + h);
                    for sy in rows.filter(|s| (s / self.intervals).min(last) == ty) {
                        let cols = ox.max(x)..(ox + tw).min(x + w);
                        for sx in cols.filter(|s| (s / self.intervals).min(last) == tx) {
                            let (src, dst) = ((sy - oy) * tw + sx - ox, (sy - y) * w + sx - x);
                            if index == 0 {
                                let code =
                                    u16::from_be_bytes([pixels[2 * src], pixels[2 * src + 1]]);
                                heights[dst] = (self.h_min + f64::from(code) * self.step) as f32;
                            } else {
                                masks[index - 1][dst] = pixels[src];
                            }
                        }
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(8 + w * h * (4 + self.masks.len()));
        out.extend_from_slice(&(w as u32).to_le_bytes());
        out.extend_from_slice(&(h as u32).to_le_bytes());
        heights
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        masks.iter().for_each(|m| out.extend_from_slice(m));
        Ok(out)
    }

    fn handle(&self, stream: &mut TcpStream) -> Result<()> {
        let request = request(stream)?;
        ensure(request.method == "GET", "only GET is supported")?;
        let (path, query) = request.path.split_once('?').unwrap_or((&request.path, ""));
        match path {
            "/" => respond(stream, 200, "text/html; charset=utf-8", PAGE.as_bytes()),
            "/manifest.json" => respond(
                stream,
                200,
                "application/json",
                &serde_json::to_vec(&self.manifest)?,
            ),
            "/region" => {
                let params: BTreeMap<_, _> = query
                    .split('&')
                    .filter_map(|p| p.split_once('='))
                    .filter_map(|(k, v)| Some((k, v.parse::<usize>().ok()?)))
                    .collect();
                let get = |k: &str| {
                    params
                        .get(k)
                        .copied()
                        .ok_or_else(|| AcquisitionError::Invalid(format!("missing {k}")))
                };
                let body = self.region(get("level")?, get("x")?, get("y")?, get("side")?)?;
                respond(stream, 200, "application/octet-stream", &body)
            }
            _ => respond(stream, 404, "text/plain", b"Not found"),
        }
    }
}

/// Width, height and raw big-endian samples of a grayscale tile.
fn read_png(path: &Path) -> Result<(usize, usize, Vec<u8>)> {
    let file = io_context("open tile", fs::File::open(path))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .map_err(|e| AcquisitionError::Invalid(format!("{}: {e}", path.display())))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| AcquisitionError::Invalid(format!("{}: tile too large", path.display())))?;
    let mut pixels = vec![0; size];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|e| AcquisitionError::Invalid(format!("{}: {e}", path.display())))?;
    pixels.truncate(info.buffer_size());
    Ok((info.width as usize, info.height as usize, pixels))
}

/// Serve the viewer until interrupted. Port zero chooses an available loopback port.
pub fn view(export: &Path, port: u16) -> Result<()> {
    let export = Export::open(export)?;
    let listener = io_context(
        "bind viewer loopback",
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)),
    )?;
    let authority = io_context("read viewer address", listener.local_addr())?;
    println!("http://{authority}/");
    io_context("flush viewer URL", std::io::stdout().flush())?;
    for stream in listener.incoming() {
        let mut stream = io_context("accept viewer connection", stream)?;
        if let Err(error) = export.handle(&mut stream) {
            let _ = respond(&mut stream, 400, "text/plain", error.to_string().as_bytes());
        }
    }
    Ok(())
}
