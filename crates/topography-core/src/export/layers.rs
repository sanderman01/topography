//! Export layer table and per-layer manifest metadata.

use serde_json::{Value, json};

/// Texels each non-height tile repeats from its neighbours on every side.
pub(super) const APRON: usize = 4;

#[derive(Clone, Copy)]
pub(super) struct LayerSpec {
    pub name: &'static str,
    pub prefix: &'static str,
    pub sixteen_bit: bool,
    pub scale: f64,
    pub offset: f64,
    pub unit: &'static str,
    pub finest_level: u32,
    pub filter: &'static str,
    /// "generated", "hydrology" or "procedural".
    pub source: &'static str,
    pub apron: usize,
    pub missing: &'static str,
    pub legend: Option<&'static str>,
}

const NEVER: &str = "never missing";
const DRY: &str = "no water in this tile";

/// Scale and offset are the height encoding's step and h_min, set per export.
pub(super) const HEIGHT: LayerSpec = LayerSpec {
    name: "height",
    prefix: "height",
    sixteen_bit: true,
    scale: f64::NAN,
    offset: f64::NAN,
    unit: "m",
    finest_level: 0,
    filter: "area mean",
    source: "generated",
    apron: 0,
    missing: NEVER,
    legend: None,
};

pub(super) const WATER_SURFACE: LayerSpec = LayerSpec {
    name: "water-surface",
    prefix: "water-surface",
    filter: "area mean within one body; lakes and sea at their level",
    source: "hydrology",
    apron: APRON,
    missing: DRY,
    ..HEIGHT
};

pub(super) const WATER_COVERAGE: LayerSpec = LayerSpec {
    name: "water-coverage",
    prefix: "water-coverage",
    sixteen_bit: false,
    scale: 1.0 / 255.0,
    offset: 0.0,
    unit: "fraction",
    filter: "area mean",
    ..WATER_SURFACE
};

pub(super) const WATER_BODY: LayerSpec = LayerSpec {
    name: "water-body",
    prefix: "water-body",
    sixteen_bit: true,
    scale: 1.0,
    unit: "id",
    filter: "none: centre body if wet, else the main wet body",
    ..WATER_COVERAGE
};

/// Finest level is the world's game level, set per export.
pub(super) const BIOME: LayerSpec = LayerSpec {
    name: "biome",
    prefix: "biome",
    sixteen_bit: false,
    scale: 1.0,
    offset: 0.0,
    unit: "class id",
    finest_level: 1,
    filter: "area mode",
    source: "procedural",
    apron: APRON,
    missing: NEVER,
    legend: Some("biome"),
};

pub(super) const LAND_COVER: LayerSpec = LayerSpec {
    name: "land-cover",
    prefix: "landcover",
    unit: "WorldCover code",
    legend: Some("land-cover"),
    ..BIOME
};

pub(super) const VEGETATION_DENSITY: LayerSpec = LayerSpec {
    name: "vegetation-density",
    prefix: "vegetation-density",
    scale: 1.0 / 255.0,
    unit: "fraction",
    filter: "area mean",
    legend: None,
    ..BIOME
};

pub(super) const SNOW_COVER: LayerSpec = LayerSpec {
    name: "snow-cover",
    prefix: "snow-cover",
    ..VEGETATION_DENSITY
};

pub(super) const TALUS_DENSITY: LayerSpec = LayerSpec {
    name: "talus-density",
    prefix: "talus-density",
    ..VEGETATION_DENSITY
};

/// Finest level is the moisture level, set per export.
pub(super) const MOISTURE: LayerSpec = LayerSpec {
    name: "moisture",
    prefix: "moisture",
    ..VEGETATION_DENSITY
};

/// Class layers (mode filter) and density layers (mean filter) of the world's `game/`.
pub(super) const GAME_CLASSES: [LayerSpec; 2] = [BIOME, LAND_COVER];
pub(super) const GAME_DENSITIES: [LayerSpec; 3] = [VEGETATION_DENSITY, SNOW_COVER, TALUS_DENSITY];

pub(super) fn metadata(spec: &LayerSpec, level0_spacing_m: f64, last_level: u32) -> Value {
    debug_assert!(
        spec.scale.is_finite() && spec.offset.is_finite(),
        "{}",
        spec.name
    );
    let mut m = json!({
        "prefix": spec.prefix,
        "format": if spec.sixteen_bit { "PNG grayscale 16-bit" } else { "PNG grayscale 8-bit" },
        "scale": spec.scale,
        "offset": spec.offset,
        "decode": "value = offset + code · scale",
        "unit": spec.unit,
        "finest_spacing_m": level0_spacing_m * f64::from(1u32 << spec.finest_level),
        "levels": [spec.finest_level, last_level],
        "filter": spec.filter,
        "source": spec.source,
        "apron_texels": spec.apron,
        "missing": spec.missing,
    });
    if let Some(legend) = spec.legend {
        m["legend"] = json!(legend);
    }
    if spec.name == WATER_SURFACE.name {
        m["extended_texels"] = json!(APRON);
        m["beyond_extension"] = json!("beyond the extension the surface is the ground height");
    }
    m
}
