//! Single-user loopback gallery with explicit votes and background technical preparation.
use super::gallery::{self, Gallery};
use crate::{
    acquisition::{AcquisitionError, Result, digest, ensure, hash_file, io_context},
    catalog::curation,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
fn request(stream: &mut TcpStream) -> Result<Request> {
    io_context(
        "set read timeout",
        stream.set_read_timeout(Some(Duration::from_secs(2))),
    )?;
    io_context(
        "set write timeout",
        stream.set_write_timeout(Some(Duration::from_secs(5))),
    )?;
    let start = Instant::now();
    let mut bytes = Vec::new();
    let mut part = [0u8; 1024];
    let split = loop {
        ensure(start.elapsed() < Duration::from_secs(5), "request timeout")?;
        let n = io_context("read request", stream.read(&mut part))?;
        ensure(n > 0, "incomplete request")?;
        bytes.extend_from_slice(&part[..n]);
        ensure(bytes.len() <= 16384, "request too large")?;
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    ensure(split <= 8192, "headers too large")?;
    let text = std::str::from_utf8(&bytes[..split])
        .map_err(|_| AcquisitionError::Invalid("invalid request headers".into()))?;
    let mut lines = text.split("\r\n");
    let first = lines
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>();
    ensure(
        first.len() == 3 && first[2] == "HTTP/1.1",
        "HTTP/1.1 required",
    )?;
    let method = first[0].to_owned();
    let path = first[1].to_owned();
    let mut headers = BTreeMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| AcquisitionError::Invalid("malformed header".into()))?;
        ensure(
            !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid header",
        )?;
        ensure(
            headers
                .insert(name.to_ascii_lowercase(), value.trim().to_owned())
                .is_none(),
            "duplicate header",
        )?;
    }
    ensure(
        !headers.contains_key("transfer-encoding"),
        "chunked requests unsupported",
    )?;
    let length = match headers.get("content-length") {
        Some(s) => s
            .parse::<usize>()
            .map_err(|_| AcquisitionError::Invalid("invalid content length".into()))?,
        None => 0,
    };
    ensure(length <= 4096, "request body too large")?;
    ensure(bytes.len() <= split + length, "extra request bytes")?;
    while bytes.len() < split + length {
        ensure(start.elapsed() < Duration::from_secs(5), "request timeout")?;
        let remaining = (split + length - bytes.len()).min(part.len());
        let n = io_context("read request body", stream.read(&mut part[..remaining]))?;
        ensure(n > 0, "incomplete body")?;
        bytes.extend_from_slice(&part[..n]);
    }
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[split..].to_vec(),
    })
}
fn respond(stream: &mut TcpStream, code: u16, kind: &str, bytes: &[u8]) -> Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let headers = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:; frame-src 'self'; connect-src 'self'; frame-ancestors 'self'; object-src 'none'; base-uri 'none'\r\n\r\n",
        bytes.len()
    );
    io_context("write response", stream.write_all(headers.as_bytes()))?;
    io_context("write response body", stream.write_all(bytes))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vote {
    index: usize,
    gallery_sha256: String,
    review_sha256: String,
    previous_record_sha256: Option<String>,
    decision: String,
}
/// Per-tile artifact check results; None while pending.
type ArtifactResults = Arc<Mutex<Vec<Option<std::result::Result<Value, String>>>>>;
struct Session {
    gallery: Arc<Gallery>,
    snapshots: Arc<Vec<super::bundle::Bundle>>,
    preparation: super::preparation_worker::Worker,
    artifacts: ArtifactResults,
    locations: Vec<Value>,
    id: String,
    root: PathBuf,
    runtime: PathBuf,
    authority: String,
    token: String,
}
impl Session {
    fn state(&self) -> Result<Value> {
        let selections = self
            .gallery
            .tiles
            .iter()
            .zip(self.snapshots.iter())
            .map(|(tile, snapshot)| {
                curation::selection(&self.runtime, &tile.review_sha256, snapshot)
            })
            .collect::<Result<Vec<_>>>()?;
        let heads = self
            .gallery
            .tiles
            .iter()
            .zip(&selections)
            .map(|(tile, state)| {
                (
                    tile.review_sha256.clone(),
                    state["head"].as_str().map(str::to_owned),
                )
            })
            .collect();
        let mut tiles = Vec::new();
        for (index, tile) in self.gallery.tiles.iter().enumerate() {
            let state = &selections[index];
            let preparation = self.preparation.state(index, state, &heads);
            tiles.push(json!({"index":index,"title":tile.title,"id":tile.id,"primary_class_id":tile.primary_class_id,"review_sha256":tile.review_sha256,"head":state["head"],"decision":state["decision"],"preparation":preparation,"location":self.locations[index]}));
        }
        Ok(json!({"title":self.gallery.title,"gallery_sha256":self.id,"tiles":tiles}))
    }
    /// Read-only cover categories for a tile whose current product is v2 Ready.
    fn overlay(&self, index: usize) -> Result<Option<Value>> {
        let state = self.state()?;
        let preparation = &state["tiles"][index]["preparation"];
        let Some(manifest) = preparation["manifest"].as_str() else {
            return Ok(None);
        };
        if preparation["state"] != "ready" {
            return Ok(None);
        }
        let [_, _, width, height] = self.snapshots[index].manifest.window_cells;
        super::overlay::render(
            Path::new(manifest),
            &self.gallery.tiles[index].review_sha256,
            [width, height],
        )
        .map(Some)
    }
    fn handle(&self, stream: &mut TcpStream, request: Request) -> Result<()> {
        ensure(
            request.headers.get("host") == Some(&self.authority),
            "invalid Host",
        )?;
        if request.method == "GET" && request.body.is_empty() {
            if request.path == "/" {
                let config = serde_json::to_string(&json!({"token":self.token}))?;
                let html = include_str!("gallery.html").replace("__GALLERY_CONFIG__", &config);
                return respond(stream, 200, "text/html; charset=utf-8", html.as_bytes());
            }
            if request.path == "/api/tiles" {
                return respond(
                    stream,
                    200,
                    "application/json",
                    &serde_json::to_vec(&self.state()?)?,
                );
            }
            if let Some(index) = request
                .path
                .strip_prefix("/api/artifacts/")
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|i| *i < self.gallery.tiles.len())
            {
                let entry = self.artifacts.lock().ok().and_then(|a| a[index].clone());
                let (code, body) = match entry {
                    Some(Ok(value)) => (200, value),
                    Some(Err(error)) => (409, json!({"error": error})),
                    None => (202, json!({"pending": true})),
                };
                return respond(
                    stream,
                    code,
                    "application/json",
                    &serde_json::to_vec(&body)?,
                );
            }
            if let Some(index) = request
                .path
                .strip_prefix("/api/overlay/")
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|i| *i < self.gallery.tiles.len())
            {
                let (code, body) = match self.overlay(index) {
                    Ok(Some(value)) => (200, value),
                    Ok(None) => (
                        404,
                        json!({"error":"Cover overlays need a ready v2 prepared product."}),
                    ),
                    Err(e) => (409, json!({"error":e.to_string()})),
                };
                return respond(
                    stream,
                    code,
                    "application/json",
                    &serde_json::to_vec(&body)?,
                );
            }
            for (prefix, name, kind) in [
                ("/preview/", "preview", "text/html; charset=utf-8"),
                ("/thumb/", "thumb", "image/png"),
            ] {
                if let Some(suffix) = request.path.strip_prefix(prefix) {
                    if let Ok(index) = suffix.parse::<usize>() {
                        if let Some(tile) = self.gallery.tiles.get(index) {
                            let ext = if name == "preview" { "html" } else { "png" };
                            let path = self.root.join(format!("{name}-{index}.{ext}"));
                            let (expected, limit) = if name == "preview" {
                                (&tile.preview_sha256, 128 * 1024 * 1024)
                            } else {
                                (&tile.thumbnail_sha256, 1024 * 1024)
                            };
                            ensure(
                                hash_file(&path)?.0 == *expected,
                                "gallery display integrity mismatch",
                            )?;
                            let file = io_context("open display", fs::File::open(path))?;
                            let mut bytes = Vec::new();
                            io_context(
                                "read display",
                                file.take(limit + 1).read_to_end(&mut bytes),
                            )?;
                            ensure(
                                bytes.len() as u64 <= limit && digest(&bytes) == *expected,
                                "gallery display changed",
                            )?;
                            if name == "preview" {
                                let identity = json!({"index":index,"review":tile.review_sha256});
                                let bridge = include_str!("gallery_bridge.html")
                                    .replace("__GALLERY_IDENTITY__", &identity.to_string());
                                bytes.extend_from_slice(bridge.as_bytes());
                            }
                            return respond(stream, 200, kind, &bytes);
                        }
                    }
                }
            }
            return respond(stream, 404, "text/plain", b"Not found");
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
            let result = (|| {
                ensure(vote.gallery_sha256 == self.id, "gallery identity mismatch")?;
                let tile = self
                    .gallery
                    .tiles
                    .get(vote.index)
                    .ok_or_else(|| AcquisitionError::Invalid("unknown tile".into()))?;
                ensure(
                    vote.review_sha256 == tile.review_sha256,
                    "review identity mismatch",
                )?;
                ensure(
                    hash_file(&self.root.join(format!("preview-{}.html", vote.index)))?.0
                        == tile.preview_sha256,
                    "reviewed preview changed",
                )?;
                ensure(
                    digest(&crate::acquisition::read_small(
                        &self.root.join("legend.toml"),
                    )?) == self.gallery.legend_sha256,
                    "gallery legend changed",
                )?;
                curation::select(
                    &self.runtime,
                    tile,
                    &self.root.join("legend.toml"),
                    vote.previous_record_sha256,
                    &vote.decision,
                    &self.id,
                    &self.snapshots[vote.index],
                )
            })();
            return match result {
                Ok(value) => respond(
                    stream,
                    200,
                    "application/json",
                    &serde_json::to_vec(&value)?,
                ),
                Err(e) => respond(
                    stream,
                    409,
                    "application/json",
                    &serde_json::to_vec(&json!({"error":e.to_string()}))?,
                ),
            };
        }
        respond(stream, 404, "text/plain", b"Not found")
    }
}
/// Serve a gallery until interrupted. Port zero chooses an available loopback port.
pub fn serve(runtime: &Path, manifest: &Path, port: u16) -> Result<()> {
    crate::config::inspect_runtime(runtime)?;
    // Unchanged files are not re-hashed on every start.
    crate::acquisition::hash_memo::enable(
        &super::preparation_worker::review_root(runtime)?.join("file-digests.json"),
    );
    let loaded = gallery::load(manifest);
    crate::acquisition::hash_memo::save();
    let (gallery, id, snapshots) = loaded?;
    crate::acquisition::geofabrik::use_runtime(runtime)?;
    let listener = io_context(
        "bind gallery loopback",
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)),
    )?;
    let authority = io_context("read gallery address", listener.local_addr())?.to_string();
    let mut entropy = [0u8; 32];
    io_context(
        "read gallery session entropy",
        fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut entropy)),
    )?;
    let gallery = Arc::new(gallery);
    let snapshots = Arc::new(snapshots);
    for tile in &gallery.tiles {
        if let Some(previous) = &tile.predecessor {
            // Identical terrain under complete metadata keeps the earlier vote.
            if let Err(error) = curation::carry_metadata_successor(
                runtime,
                &previous.raw,
                &previous.review,
                &tile.raw,
                &tile.review,
            ) {
                eprintln!("Metadata successor {}: {error}", tile.id);
            }
        }
    }
    for (tile, snapshot) in gallery.tiles.iter().zip(snapshots.iter()) {
        curation::selection(runtime, &tile.review_sha256, snapshot)?;
    }
    // Coordinates are display aids; a failure leaves them out.
    let locations = gallery
        .tiles
        .iter()
        .zip(snapshots.iter())
        .map(|(tile, snapshot)| super::location(&tile.review, snapshot).unwrap_or(Value::Null))
        .collect();
    let root = manifest.parent().unwrap_or(Path::new("."));
    let preparation = super::preparation_worker::start(
        runtime,
        &root.join("legend.toml"),
        gallery.clone(),
        snapshots.clone(),
    )?;
    let artifacts = Arc::new(Mutex::new(vec![None; gallery.tiles.len()]));
    let cache = super::preparation_worker::review_root(runtime)?.join("artifact-checks");
    start_artifact_checks(gallery.clone(), snapshots.clone(), artifacts.clone(), cache)?;
    let session = Session {
        gallery,
        snapshots,
        preparation,
        artifacts,
        locations,
        id,
        root: root.into(),
        runtime: runtime.into(),
        authority,
        token: digest(&entropy),
    };
    // Fail before announcing a usable server if the existing catalog is damaged.
    session.state()?;
    println!("http://{}/", session.authority);
    io_context("flush gallery URL", std::io::stdout().flush())?;
    for stream in listener.incoming() {
        let mut stream = io_context("accept gallery connection", stream)?;
        let result = request(&mut stream).and_then(|r| session.handle(&mut stream, r));
        if let Err(error) = result {
            let _ = respond(
                &mut stream,
                400,
                "application/json",
                &serde_json::to_vec(&json!({"error":error.to_string()}))?,
            );
        }
    }
    Ok(())
}

/// Artifact checks of each review crop, computed once in the background (display aid)
/// and cached per review and detector version.
fn start_artifact_checks(
    gallery: Arc<Gallery>,
    snapshots: Arc<Vec<super::bundle::Bundle>>,
    results: ArtifactResults,
    cache: PathBuf,
) -> Result<()> {
    io_context(
        "start artifact checks",
        std::thread::Builder::new()
            .name("terrain-artifacts".into())
            .spawn(move || {
                for (index, (tile, snapshot)) in
                    gallery.tiles.iter().zip(snapshots.iter()).enumerate()
                {
                    let path = cache.join(format!(
                        "{}-{}.json",
                        tile.review_sha256,
                        super::artifacts::CHECKS
                    ));
                    // Our own cache of a 2048² mask: larger than `read_small` allows.
                    let cached = fs::read(&path)
                        .ok()
                        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
                    let result = match cached {
                        Some(cached) => Ok(cached),
                        None => review_artifacts(&tile.review, snapshot).inspect(|value| {
                            // The cache only saves time; a failed write is harmless.
                            let _ = fs::create_dir_all(&cache).and_then(|()| {
                                let mut temp = tempfile::NamedTempFile::new_in(&cache)?;
                                temp.write_all(&serde_json::to_vec(value)?)?;
                                temp.persist(&path).map(|_| ()).map_err(|e| e.error)
                            });
                        }),
                    }
                    .map_err(|e| e.to_string());
                    if let Ok(mut results) = results.lock() {
                        results[index] = Some(result);
                    }
                }
            }),
    )?;
    Ok(())
}

fn review_artifacts(review: &Path, snapshot: &super::bundle::Bundle) -> Result<Value> {
    let [_, _, w, h] = snapshot.manifest.window_cells;
    let (w, h) = (w as usize, h as usize);
    let dir = review.parent().unwrap_or(Path::new("."));
    let scratch = io_context("stage artifact check", tempfile::tempdir())?;
    let elevation = super::band(
        &dir.join("elevation.tif"),
        scratch.path(),
        "elevation",
        w * h,
        true,
    )?;
    let validity = super::band(
        &dir.join("validity.tif"),
        scratch.path(),
        "validity",
        w * h,
        false,
    )?;
    let height: Vec<f32> = elevation
        .chunks_exact(4)
        .zip(&validity)
        .map(|(b, v)| {
            let value = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if *v != 0 { value } else { f32::NAN }
        })
        .collect();
    let transform: Vec<f64> =
        serde_json::from_value(snapshot.inspection["geoTransform"].clone()).unwrap_or_default();
    let spacing = [
        transform.get(1).copied().unwrap_or(1.0).abs(),
        transform.get(5).copied().unwrap_or(-1.0).abs(),
    ];
    let detection = super::artifacts::detect_metre(&height, w, h, spacing[0]);
    Ok(json!({
        "width": w,
        "height": h,
        "cells": super::base64(&detection.mask),
        "summary": super::artifacts::summary(&height, &detection, spacing),
    }))
}
