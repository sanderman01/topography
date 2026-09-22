//! Pinned gallery publication and direct whole-tile curation.
use crate::{
    acquisition::{
        Result, digest, ensure, hash_file, io_context, raster, read_json, read_small, write_new,
    },
    catalog,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema_version: u32,
    title: String,
    tiles: Vec<InputTile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputTile {
    id: String,
    title: String,
    raw: PathBuf,
    review: PathBuf,
    primary_class_id: u16,
    preparation_profile: Option<InputProfile>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum InputProfile {
    Pinned(ProfileRef),
    Path(PathBuf),
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileRef {
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tile {
    pub id: String,
    pub title: String,
    pub raw: PathBuf,
    pub raw_sha256: String,
    pub review: PathBuf,
    pub review_sha256: String,
    pub primary_class_id: u16,
    pub preview_sha256: String,
    pub preview_bytes: u64,
    pub thumbnail_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation_profile: Option<ProfileRef>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gallery {
    pub schema_version: u32,
    pub kind: String,
    pub title: String,
    pub legend_sha256: String,
    pub tiles: Vec<Tile>,
}

/// Publish a pinned gallery; this records no votes and never mutates source previews.
pub fn build(input: &Path, legend: &Path, output: &Path) -> Result<PathBuf> {
    ensure(!output.exists(), "gallery output already exists")?;
    let request: Input = read_json(input)?;
    ensure(
        request.schema_version == 1 && !request.tiles.is_empty() && request.tiles.len() <= 2048,
        "gallery needs version 1 and 1..=2048 tiles",
    )?;
    ensure(
        !request.title.trim().is_empty() && request.title.len() <= 240,
        "invalid gallery title",
    )?;
    let active = catalog::inspect_legend(legend)?;
    let base = input.parent().unwrap_or(Path::new("."));
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    io_context("create gallery parent", fs::create_dir_all(parent))?;
    let stage = io_context("stage gallery", tempfile::tempdir_in(parent))?;
    let mut ids = BTreeSet::new();
    let mut subjects = BTreeSet::new();
    let mut tiles = Vec::new();
    let mut raw_cache = BTreeMap::new();
    for (index, tile) in request.tiles.into_iter().enumerate() {
        ensure(
            !tile.id.is_empty() && tile.id.len() <= 180 && ids.insert(tile.id.clone()),
            "invalid or duplicate gallery ID",
        )?;
        ensure(
            !tile.title.trim().is_empty() && tile.title.len() <= 240,
            "invalid tile title",
        )?;
        ensure(
            active
                .classes
                .iter()
                .any(|(id, _)| *id == tile.primary_class_id),
            "gallery class is not active",
        )?;
        let raw = io_context("resolve gallery raw", fs::canonicalize(base.join(tile.raw)))?;
        let review = io_context(
            "resolve gallery review",
            fs::canonicalize(base.join(tile.review)),
        )?;
        if !raw_cache.contains_key(&raw) {
            raw_cache.insert(raw.clone(), crate::acquisition::verify(&raw)?);
        }
        let bundle = super::bundle::verify_with_raw(&raw, &review, &raw_cache[&raw])?;
        let review_sha256 = digest(&read_small(&review)?);
        ensure(
            subjects.insert(review_sha256.clone()),
            "duplicate gallery review",
        )?;
        let preview = bundle
            .manifest
            .artifacts
            .iter()
            .find(|a| a.path == "preview.html")
            .ok_or_else(|| {
                crate::acquisition::AcquisitionError::Invalid("review has no preview.html".into())
            })?;
        let directory = review.parent().unwrap_or(Path::new("."));
        let copied = stage.path().join(format!("preview-{index}.html"));
        io_context(
            "copy exact preview",
            fs::copy(directory.join("preview.html"), &copied),
        )?;
        ensure(
            hash_file(&copied)? == (preview.sha256.clone(), preview.bytes),
            "preview changed during gallery build",
        )?;
        let small = stage.path().join("thumbnail-source.tif");
        let [_, _, width, height] = bundle.manifest.window_cells;
        let scale = (192.0 / f64::from(width.max(height))).min(1.0);
        let width = ((f64::from(width) * scale).round() as u32)
            .max(1)
            .to_string();
        let height = ((f64::from(height) * scale).round() as u32)
            .max(1)
            .to_string();
        raster::run(
            raster::command("gdal_translate")
                .args([
                    "-q", "-of", "GTiff", "-outsize", &width, &height, "-r", "average",
                ])
                .arg(directory.join("elevation.tif"))
                .arg(&small),
        )?;
        let thumb = stage.path().join(format!("thumb-{index}.png"));
        raster::run(
            raster::command("gdaldem")
                .args(["hillshade", "-q", "-compute_edges", "-of", "PNG"])
                .arg(&small)
                .arg(&thumb),
        )?;
        tiles.push(Tile {
            id: tile.id,
            title: tile.title,
            raw_sha256: digest(&read_small(&raw)?),
            raw,
            review,
            review_sha256,
            primary_class_id: tile.primary_class_id,
            preview_sha256: preview.sha256.clone(),
            preview_bytes: preview.bytes,
            thumbnail_sha256: hash_file(&thumb)?.0,
            preparation_profile: tile
                .preparation_profile
                .map(|input| {
                    let (path, expected) = match input {
                        InputProfile::Path(path) => (path, None),
                        InputProfile::Pinned(profile) => (profile.path, Some(profile.sha256)),
                    };
                    let path = io_context(
                        "resolve preparation profile",
                        fs::canonicalize(base.join(path)),
                    )?;
                    let sha256 = digest(&read_small(&path)?);
                    ensure(
                        expected.is_none_or(|expected| expected == sha256),
                        "preparation profile differs from batch input",
                    )?;
                    Ok::<_, crate::acquisition::AcquisitionError>(ProfileRef { sha256, path })
                })
                .transpose()?,
        });
    }
    let legend_bytes = read_small(legend)?;
    write_new(&stage.path().join("legend.toml"), &legend_bytes)?;
    let gallery = Gallery {
        schema_version: 1,
        kind: "terrain_review_gallery".into(),
        title: request.title,
        legend_sha256: digest(&legend_bytes),
        tiles,
    };
    let manifest = serde_json::to_vec_pretty(&gallery)?;
    ensure(
        manifest.len() <= 1024 * 1024,
        "gallery manifest exceeds 1 MiB; split the input into smaller batches",
    )?;
    write_new(&stage.path().join("gallery.json"), &manifest)?;
    io_context("publish gallery", fs::rename(stage.path(), output))?;
    Ok(output.join("gallery.json"))
}

pub(crate) fn load(path: &Path) -> Result<(Gallery, String, Vec<super::bundle::Bundle>)> {
    let bytes = read_small(path)?;
    let gallery: Gallery = serde_json::from_slice(&bytes)?;
    ensure(
        gallery.schema_version == 1
            && gallery.kind == "terrain_review_gallery"
            && !gallery.tiles.is_empty()
            && gallery.tiles.len() <= 2048,
        "unsupported gallery",
    )?;
    let base = path.parent().unwrap_or(Path::new("."));
    ensure(
        digest(&read_small(&base.join("legend.toml"))?) == gallery.legend_sha256,
        "gallery legend integrity mismatch",
    )?;
    let mut raw_cache = BTreeMap::new();
    let mut snapshots = Vec::new();
    for (index, tile) in gallery.tiles.iter().enumerate() {
        if let Some(profile) = &tile.preparation_profile {
            ensure(
                crate::acquisition::valid_digest(&profile.sha256) && profile.path.is_absolute(),
                "invalid preparation profile reference",
            )?;
        }
        ensure(
            digest(&read_small(&tile.raw)?) == tile.raw_sha256
                && digest(&read_small(&tile.review)?) == tile.review_sha256,
            "gallery input identity mismatch",
        )?;
        if !raw_cache.contains_key(&tile.raw) {
            raw_cache.insert(tile.raw.clone(), crate::acquisition::verify(&tile.raw)?);
        }
        snapshots.push(super::bundle::verify_with_raw(
            &tile.raw,
            &tile.review,
            &raw_cache[&tile.raw],
        )?);
        ensure(
            tile.preview_bytes <= 128 * 1024 * 1024
                && hash_file(&base.join(format!("preview-{index}.html")))?
                    == (tile.preview_sha256.clone(), tile.preview_bytes),
            "gallery preview integrity mismatch",
        )?;
        ensure(
            hash_file(&base.join(format!("thumb-{index}.png")))?.0 == tile.thumbnail_sha256,
            "gallery thumbnail integrity mismatch",
        )?;
    }
    Ok((gallery, digest(&bytes), snapshots))
}
