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

/// 8-bit layers served after the heights, in this order; absent layers are zeros.
const SERVED: [&str; 5] = [
    "water-coverage",
    "snow-cover",
    "vegetation-density",
    "land-cover",
    "biome",
];

/// One layer's tile naming and level range from the manifest.
struct Layer {
    name: String,
    prefix: String,
    finest: usize,
    last: usize,
    apron: usize,
    /// Tiles may be absent (water layers); absent texels read as zero.
    sparse: bool,
}

struct Export {
    root: PathBuf,
    manifest: Value,
    intervals: usize,
    h_min: f64,
    step: f64,
    height: Layer,
    served: Vec<Option<Layer>>,
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
        ensure(
            manifest["schema_version"] == 2 || manifest["schema_version"] == 3,
            "export manifest is not schema version 2 or 3: re-export with this version to view",
        )?;
        let number = |v: &Value, what: &str| {
            v.as_f64()
                .ok_or_else(|| AcquisitionError::Invalid(format!("export manifest lacks {what}")))
        };
        let intervals = number(&manifest["intervals"], "intervals")? as usize;
        let h_min = number(&manifest["encoding"]["h_min_m"], "h_min_m")?;
        let step = number(&manifest["encoding"]["step_m"], "step_m")?;
        let layer = |name: &str| -> Result<Option<Layer>> {
            let m = &manifest["layers"][name];
            if m.is_null() {
                return Ok(None);
            }
            let level = |i: usize| {
                m["levels"][i].as_u64().map(|n| n as usize).ok_or_else(|| {
                    AcquisitionError::Invalid(format!("export layer {name} lacks levels"))
                })
            };
            let prefix = m["prefix"].as_str().ok_or_else(|| {
                AcquisitionError::Invalid(format!("export layer {name} lacks prefix"))
            })?;
            Ok(Some(Layer {
                name: name.into(),
                prefix: prefix.into(),
                finest: level(0)?,
                last: level(1)?,
                apron: m["apron_texels"].as_u64().unwrap_or(0) as usize,
                sparse: m["missing"] != "never missing",
            }))
        };
        let height = layer("height")?
            .ok_or_else(|| AcquisitionError::Invalid("export lacks the height layer".into()))?;
        let served = SERVED.iter().map(|n| layer(n)).collect::<Result<_>>()?;
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
            height,
            served,
            levels,
        })
    }

    /// Codes of `layer` for samples [x, x+w) × [y, y+h) of `level`, read at the layer's
    /// finest level when that is coarser (nearest texel). Absent texels are zero.
    fn read(
        &self,
        layer: &Layer,
        level: usize,
        (x, y, w, h): (usize, usize, usize, usize),
    ) -> Result<Vec<u16>> {
        let mut out = vec![0u16; w * h];
        if level > layer.last {
            return Ok(out);
        }
        let src = level.max(layer.finest);
        let samples = self.levels[src];
        // Texel i of level k sits at level-0 sample i·2^k.
        let shift = src - level;
        let map = |s: usize| ((s + ((1 << shift) >> 1)) >> shift).min(samples - 1);
        let (xs, ys): (Vec<_>, Vec<_>) =
            ((x..x + w).map(map).collect(), (y..y + h).map(map).collect());
        let n = self.intervals;
        let last = samples.div_ceil(n) - 1;
        let tile = |s: usize| (s / n).min(last);
        let a = layer.apron;
        for ty in tile(ys[0])..=tile(ys[h - 1]) {
            for tx in tile(xs[0])..=tile(xs[w - 1]) {
                let path = self.root.join(format!(
                    "{}/L{src}/{}_L{src}_{tx}_{ty}.png",
                    layer.name, layer.prefix
                ));
                if layer.sparse && !path.exists() {
                    continue;
                }
                let (tw, th, pixels) = read_png(&path)?;
                let bytes = pixels.len() / (tw * th);
                let (ox, oy) = (tx * n, ty * n);
                for (dy, &sy) in ys.iter().enumerate().filter(|(_, s)| tile(**s) == ty) {
                    for (dx, &sx) in xs.iter().enumerate().filter(|(_, s)| tile(**s) == tx) {
                        let i = (sy - oy + a) * tw + sx - ox + a;
                        // RGB class tiles: the dominant class (R).
                        out[dy * w + dx] = match bytes {
                            2 => u16::from_be_bytes([pixels[2 * i], pixels[2 * i + 1]]),
                            3 => u16::from(pixels[3 * i]),
                            _ => u16::from(pixels[i]),
                        };
                    }
                }
            }
        }
        Ok(out)
    }

    /// Heights (f32 m) then the `SERVED` layers (u8 each) for samples [x, x+w) × [y, y+h)
    /// of one level.
    fn region(&self, level: usize, x: usize, y: usize, side: usize) -> Result<Vec<u8>> {
        let samples = *self
            .levels
            .get(level)
            .ok_or_else(|| AcquisitionError::Invalid("unknown level".into()))?;
        ensure(side > 0 && side <= MAX_SIDE, "region too large")?;
        ensure(x < samples && y < samples, "region outside the export")?;
        let rect = (x, y, side.min(samples - x), side.min(samples - y));
        let (w, h) = (rect.2, rect.3);
        let mut out = Vec::with_capacity(8 + w * h * (4 + SERVED.len()));
        out.extend_from_slice(&(w as u32).to_le_bytes());
        out.extend_from_slice(&(h as u32).to_le_bytes());
        for code in self.read(&self.height, level, rect)? {
            let metres = (self.h_min + f64::from(code) * self.step) as f32;
            out.extend_from_slice(&metres.to_le_bytes());
        }
        for layer in &self.served {
            match layer {
                Some(layer) => {
                    let codes = self.read(layer, level, rect)?;
                    out.extend(codes.iter().map(|&c| c.min(255) as u8));
                }
                None => out.resize(out.len() + w * h, 0),
            }
        }
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
pub(super) fn read_png(path: &Path) -> Result<(usize, usize, Vec<u8>)> {
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
