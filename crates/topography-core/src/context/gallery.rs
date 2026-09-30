//! Loopback Yes/No review of context regions. Votes append to `decisions.jsonl` beside the
//! regions, keyed by the region manifest digest, so a rebuilt region needs a new vote.

use super::{DISPLAY_CELLS, LAYERS, display, heights, hillshade, regions};
use crate::{
    acquisition::{AcquisitionError, Result, digest, ensure, io_context, read_small},
    review::{
        base64,
        gallery_server::{request, respond},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const LEGEND: &str = include_str!("../../../../catalog/labels/terrain-v2.toml");

struct Entry {
    dir: PathBuf,
    sha256: String,
    manifest: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vote {
    index: usize,
    region_sha256: String,
    decision: String,
}

struct Session {
    root: PathBuf,
    entries: Vec<Entry>,
    classes: BTreeMap<i64, String>,
    /// Latest decision per region id: (region sha256, "yes" | "no").
    decisions: Mutex<BTreeMap<String, (String, String)>>,
    authority: String,
    token: String,
}

fn load_decisions(path: &Path) -> Result<BTreeMap<String, (String, String)>> {
    let mut map = BTreeMap::new();
    if !path.exists() {
        return Ok(map);
    }
    let text = io_context("read decisions", fs::read_to_string(path))?;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line)?;
        let (Some(id), Some(sha), Some(d)) = (
            v["region"].as_str(),
            v["region_sha256"].as_str(),
            v["decision"].as_str(),
        ) else {
            return Err(AcquisitionError::Invalid(format!(
                "malformed decision line: {line}"
            )));
        };
        map.insert(id.to_owned(), (sha.to_owned(), d.to_owned()));
    }
    Ok(map)
}

/// Accepted region directories under `root`: the latest vote is Yes for the current manifest.
pub fn accepted(root: &Path) -> Result<Vec<PathBuf>> {
    let decisions = load_decisions(&root.join("decisions.jsonl"))?;
    let mut out = Vec::new();
    for dir in regions(root)? {
        let sha = digest(&read_small(&dir.join("region.json"))?);
        let id = dir.file_name().unwrap_or_default().to_string_lossy();
        if decisions.get(id.as_ref()) == Some(&(sha, "yes".into())) {
            out.push(dir);
        }
    }
    Ok(out)
}

impl Session {
    fn read(&self, index: usize, name: &str) -> Result<Vec<u8>> {
        let entry = &self.entries[index];
        let bytes = io_context(format!("read {name}"), fs::read(entry.dir.join(name)))?;
        ensure(
            entry.manifest["files"][name].as_str() == Some(digest(&bytes).as_str()),
            format!("{name} changed since the gallery started"),
        )?;
        Ok(bytes)
    }

    fn state(&self) -> Result<Value> {
        let decisions = self.decisions.lock().map_err(|_| poisoned())?;
        let regions: Vec<Value> = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, e)| {
                let m = &e.manifest;
                let id = m["id"].as_str().unwrap_or_default();
                let decision = decisions
                    .get(id)
                    .filter(|(sha, _)| *sha == e.sha256)
                    .map(|(_, d)| d.clone());
                let b = &m["wgs84_bbox"];
                let centre = [
                    (b[0].as_f64().unwrap_or(0.0) + b[2].as_f64().unwrap_or(0.0)) / 2.0,
                    (b[1].as_f64().unwrap_or(0.0) + b[3].as_f64().unwrap_or(0.0)) / 2.0,
                ];
                json!({"index": index, "id": id, "region_sha256": e.sha256,
                    "class_id": m["class_id"],
                    "class": self.classes.get(&m["class_id"].as_i64().unwrap_or(0)),
                    "reference": m["reference"], "size_km": m["size"][0].as_f64().unwrap_or(0.0) * spacing(m) / 1000.0,
                    "centre": centre, "stats": m["stats"], "decision": decision})
            })
            .collect();
        Ok(json!({"regions": regions}))
    }

    /// Validity per cell: the lidar validity layer, or all valid for GLO-30.
    fn valid(&self, index: usize, cells: usize) -> Result<Vec<bool>> {
        if self.entries[index].manifest["files"]["validity.u8"].is_string() {
            Ok(self
                .read(index, "validity.u8")?
                .iter()
                .map(|v| *v == 255)
                .collect())
        } else {
            Ok(vec![true; cells])
        }
    }

    /// The raw-crop inspector page for a region, reduced for display and built on request.
    fn preview(&self, index: usize) -> Result<Vec<u8>> {
        let m = &self.entries[index].manifest;
        let n = m["size"][0].as_u64().unwrap_or(0) as usize;
        let h = heights(&self.read(index, LAYERS[0])?);
        let valid = self.valid(index, n * n)?;
        let d = display(
            &h,
            &valid,
            &self.read(index, "water.u8")?,
            n,
            spacing(m) as f32,
        );
        let (lo, hi) = d
            .height
            .iter()
            .zip(&d.valid)
            .filter(|(_, ok)| **ok)
            .fold((f32::MAX, f32::MIN), |(a, b), (v, _)| {
                (a.min(*v), b.max(*v))
            });
        let shades: Vec<String> = [315.0, 45.0, 225.0]
            .iter()
            .map(|az| base64(&hillshade(&d.height, d.n, d.n, d.spacing, *az)))
            .collect();
        let s = f64::from(d.spacing);
        let good = d.valid.iter().filter(|v| **v).count();
        let mask: Vec<u8> = d.valid.iter().map(|v| if *v { 255 } else { 0 }).collect();
        let bytes: Vec<u8> = d.height.iter().flat_map(|v| v.to_le_bytes()).collect();
        let payload = json!({"width": d.n, "height": d.n,
            "transform": [m["origin"][0], s, 0.0, m["origin"][1], 0.0, -s],
            "record_id": m["id"],
            "metrics": {"cells": d.n * d.n, "finite_valid_cells": good, "provider_invalid_cells": d.n * d.n - good,
                "nonfinite_valid_cells": 0, "minimum_native": lo, "maximum_native": hi,
                "vertical_units_declared": "metre", "vertical_datum_declared": "as published",
                "metric_north_up_preview": true},
            "elevation_le_f32": base64(&bytes), "mask": base64(&mask), "shades": shades});
        let encoded = serde_json::to_string(&payload)?.replace('<', "\\u003c");
        let identity = json!({"index": index, "review": self.entries[index].sha256});
        let mut html = include_str!("../review/preview.html").replace("__TERRAIN_DATA__", &encoded);
        html.push_str(
            &include_str!("../review/gallery_bridge.html")
                .replace("__GALLERY_IDENTITY__", &identity.to_string()),
        );
        Ok(html.into_bytes())
    }

    /// Cover overlay in the bridge's packing (2 bits each for water, snow/ice, vegetation),
    /// sampled at the preview's resolution.
    fn overlay(&self, index: usize) -> Result<Value> {
        let n = self.entries[index].manifest["size"][0]
            .as_u64()
            .unwrap_or(0) as usize;
        let f = n.div_ceil(DISPLAY_CELLS);
        let m = n / f;
        let water = self.read(index, "water.u8")?;
        let snow = self.read(index, "snow-ice.u8")?;
        let vegetation = self.read(index, "vegetation.u8")?;
        let cells: Vec<u8> = (0..m * m)
            .map(|j| {
                let i = (j / m * f + f / 2) * n + j % m * f + f / 2;
                let w = if water[i] == 255 { 2 } else { 1 };
                w | (snow[i] & 3) << 2 | (vegetation[i] & 3) << 4
            })
            .collect();
        Ok(json!({"width": m, "height": m, "cells": base64(&cells)}))
    }

    fn vote(&self, vote: Vote) -> Result<Value> {
        let entry = self
            .entries
            .get(vote.index)
            .ok_or_else(|| AcquisitionError::Invalid("unknown region".into()))?;
        ensure(
            vote.region_sha256 == entry.sha256
                && digest(&read_small(&entry.dir.join("region.json"))?) == entry.sha256,
            "region changed since the gallery started",
        )?;
        ensure(
            matches!(vote.decision.as_str(), "yes" | "no"),
            "decision must be yes or no",
        )?;
        let id = entry.manifest["id"].as_str().unwrap_or_default().to_owned();
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = json!({"region": id, "region_sha256": entry.sha256,
            "decision": vote.decision, "decided_at_unix": at, "actor": "owner via context serve"});
        let mut decisions = self.decisions.lock().map_err(|_| poisoned())?;
        let mut file = io_context(
            "open decisions",
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.root.join("decisions.jsonl")),
        )?;
        io_context(
            "append decision",
            file.write_all(format!("{line}\n").as_bytes())
                .and_then(|()| file.sync_all()),
        )?;
        decisions.insert(id, (entry.sha256.clone(), vote.decision.clone()));
        Ok(line)
    }

    fn handle(&self, stream: &mut TcpStream) -> Result<()> {
        let request = request(stream)?;
        ensure(
            request.headers.get("host") == Some(&self.authority),
            "invalid Host",
        )?;
        let index = |prefix: &str| {
            request
                .path
                .strip_prefix(prefix)
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|i| *i < self.entries.len())
        };
        let json_ok = |stream: &mut TcpStream, v: Result<Value>| match v {
            Ok(v) => respond(stream, 200, "application/json", &serde_json::to_vec(&v)?),
            Err(e) => respond(
                stream,
                409,
                "application/json",
                &serde_json::to_vec(&json!({"error": e.to_string()}))?,
            ),
        };
        if request.method == "GET" {
            if request.path == "/" {
                let config = json!({"token": self.token}).to_string();
                let html = include_str!("gallery.html").replace("__CONFIG__", &config);
                return respond(stream, 200, "text/html; charset=utf-8", html.as_bytes());
            }
            if request.path == "/api/regions" {
                return json_ok(stream, self.state());
            }
            if let Some(i) = index("/thumb/") {
                return respond(stream, 200, "image/png", &self.read(i, "overview.png")?);
            }
            if let Some(i) = index("/preview/") {
                return respond(stream, 200, "text/html; charset=utf-8", &self.preview(i)?);
            }
            if let Some(i) = index("/api/overlay/") {
                return json_ok(stream, self.overlay(i));
            }
        }
        if request.method == "POST" && request.path == "/api/decision" {
            ensure(
                request.headers.get("origin") == Some(&format!("http://{}", self.authority))
                    && request.headers.get("x-topography-token") == Some(&self.token)
                    && request
                        .headers
                        .get("content-type")
                        .is_some_and(|s| s == "application/json"),
                "same-origin JSON and session token required",
            )?;
            let vote: Vote = serde_json::from_slice(&request.body)?;
            return json_ok(stream, self.vote(vote));
        }
        respond(stream, 404, "text/plain", b"Not found")
    }
}

fn spacing(manifest: &Value) -> f64 {
    manifest["spacing_m"].as_f64().unwrap_or(32.0)
}

fn poisoned() -> AcquisitionError {
    AcquisitionError::Invalid("decision state poisoned".into())
}

/// Serve the region gallery for `root` until interrupted.
pub fn serve(root: &Path, port: u16) -> Result<()> {
    let mut entries = Vec::new();
    for dir in regions(root)? {
        let bytes = read_small(&dir.join("region.json"))?;
        let manifest: Value = serde_json::from_slice(&bytes)?;
        ensure(
            manifest["kind"] == "context_region",
            format!("{} is not a context region", dir.display()),
        )?;
        entries.push(Entry {
            dir,
            sha256: digest(&bytes),
            manifest,
        });
    }
    ensure(!entries.is_empty(), "no context regions found")?;
    let legend: Value =
        toml::from_str(LEGEND).map_err(|e| AcquisitionError::Invalid(format!("legend: {e}")))?;
    let classes = legend["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["id"].as_i64()?, c["name"].as_str()?.to_owned())))
        .collect();
    let listener = io_context(
        "bind context gallery",
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)),
    )?;
    let authority = io_context("read address", listener.local_addr())?.to_string();
    let mut entropy = [0u8; 32];
    io_context(
        "read session entropy",
        fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut entropy)),
    )?;
    let session = Session {
        root: root.into(),
        decisions: Mutex::new(load_decisions(&root.join("decisions.jsonl"))?),
        entries,
        classes,
        authority,
        token: digest(&entropy),
    };
    println!("http://{}/", session.authority);
    io_context("flush URL", std::io::stdout().flush())?;
    for stream in listener.incoming() {
        let mut stream = io_context("accept connection", stream)?;
        if let Err(error) = session.handle(&mut stream) {
            let _ = respond(
                &mut stream,
                400,
                "application/json",
                &serde_json::to_vec(&json!({"error": error.to_string()}))?,
            );
        }
    }
    Ok(())
}
