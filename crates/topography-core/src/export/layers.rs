//! Export layer table and per-layer manifest metadata.

use serde_json::{Value, json};

/// Texels each non-height tile repeats from its neighbours on every side.
pub(super) const APRON: usize = 4;

/// `water-surface` step and wrap: codes repeat every 65,536 steps.
pub(super) const WATER_STEP_M: f64 = 0.02;
pub(super) const WATER_WRAP_M: f64 = WATER_STEP_M * 65536.0;
/// Fails the export: a water surface this far from its ground cannot be unwrapped safely.
pub(super) const WATER_UNWRAP_LIMIT_M: f64 = 600.0;
// Unwrapping picks the copy nearest the ground, so the limit must stay under half the wrap.
const _: () = assert!(WATER_UNWRAP_LIMIT_M < WATER_WRAP_M / 2.0);

#[derive(Clone, Copy)]
pub(super) struct LayerSpec {
    pub name: &'static str,
    pub prefix: &'static str,
    pub sixteen_bit: bool,
    /// 1 (grayscale) or 3 (RGB class layers: a, b, weight).
    pub channels: usize,
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
    /// What the values mean, where the unit does not say it.
    pub meaning: Option<&'static str>,
}

const NEVER: &str = "never missing";
const DRY: &str = "no water in this tile";

/// Scale and offset are the height encoding's step and h_min, set per export.
pub(super) const HEIGHT: LayerSpec = LayerSpec {
    name: "height",
    prefix: "height",
    sixteen_bit: true,
    channels: 1,
    scale: f64::NAN,
    offset: f64::NAN,
    unit: "m",
    finest_level: 0,
    filter: "area mean",
    source: "generated",
    apron: 0,
    missing: NEVER,
    legend: None,
    meaning: None,
};

pub(super) const WATER_SURFACE: LayerSpec = LayerSpec {
    name: "water-surface",
    prefix: "water-surface",
    scale: WATER_STEP_M,
    offset: 0.0,
    filter: "area mean within one body; lakes and sea at their level",
    source: "hydrology",
    apron: APRON,
    missing: DRY,
    meaning: Some(
        "defined where water-body > 0 (wet texels and the extension past the shore); 0 elsewhere",
    ),
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
    meaning: Some(
        "wet share of the texel; wet ⇔ coverage > 0 (a share that rounds to 0 is written as 1)",
    ),
    ..WATER_SURFACE
};

pub(super) const WATER_BODY: LayerSpec = LayerSpec {
    name: "water-body",
    prefix: "water-body",
    sixteen_bit: true,
    scale: 1.0,
    unit: "id",
    filter: "none: centre body if wet, else the main wet body",
    meaning: Some(
        "export id of the wet body, also on extension texels (coverage 0) within 4 texels of it; 0 is no water and no surface",
    ),
    ..WATER_COVERAGE
};

/// Class layers: RGB a, b, weight. Finest level is the world's game level, set per export.
pub(super) const BIOME: LayerSpec = LayerSpec {
    name: "biome",
    prefix: "biome",
    sixteen_bit: false,
    channels: 3,
    scale: 1.0,
    offset: 0.0,
    unit: "class ids and weight",
    finest_level: 1,
    filter: "area mean of class shares, top two re-picked per level",
    source: "procedural",
    apron: APRON,
    missing: NEVER,
    legend: Some("biome"),
    meaning: Some(
        "R = dominant class a, G = runner-up class b (= a where one class), B = weight: share of a within a + b × 255 (128..255); blend a and b by B / 255",
    ),
};

pub(super) const LAND_COVER: LayerSpec = LayerSpec {
    name: "land-cover",
    prefix: "landcover",
    legend: Some("land-cover"),
    ..BIOME
};

pub(super) const VEGETATION_DENSITY: LayerSpec = LayerSpec {
    name: "vegetation-density",
    prefix: "vegetation-density",
    channels: 1,
    scale: 1.0 / 255.0,
    unit: "fraction",
    filter: "area mean",
    legend: None,
    meaning: Some(
        "canopy share of the texel: 1.0 is continuous canopy over the whole texel; closed forest reads lower (about 0.65) because forest covers only its layout share of a region and fades at its edges",
    ),
    ..BIOME
};

pub(super) const SNOW_COVER: LayerSpec = LayerSpec {
    name: "snow-cover",
    prefix: "snow-cover",
    meaning: Some("snow or ice share of the texel"),
    ..VEGETATION_DENSITY
};

pub(super) const TALUS_DENSITY: LayerSpec = LayerSpec {
    name: "talus-density",
    prefix: "talus-density",
    meaning: Some("loose rock share of the texel"),
    ..VEGETATION_DENSITY
};

/// Mean annual air temperature; code = (°C + 30) · 4.
pub(super) const TEMPERATURE: LayerSpec = LayerSpec {
    name: "temperature",
    prefix: "temperature",
    scale: 0.25,
    offset: -30.0,
    unit: "°C",
    meaning: Some("mean annual air temperature"),
    ..VEGETATION_DENSITY
};

/// Finest level is the moisture level, set per export.
pub(super) const MOISTURE: LayerSpec = LayerSpec {
    name: "moisture",
    prefix: "moisture",
    meaning: Some("0 dry to 1 wet"),
    ..VEGETATION_DENSITY
};

/// Class layers (share mean) and density-like layers (area mean) of the world's `game/`.
pub(super) const GAME_CLASSES: [LayerSpec; 2] = [BIOME, LAND_COVER];
pub(super) const GAME_DENSITIES: [LayerSpec; 4] =
    [VEGETATION_DENSITY, SNOW_COVER, TALUS_DENSITY, TEMPERATURE];

pub(super) fn metadata(spec: &LayerSpec, level0_spacing_m: f64, last_level: u32) -> Value {
    debug_assert!(
        spec.scale.is_finite() && spec.offset.is_finite(),
        "{}",
        spec.name
    );
    let format = match (spec.channels, spec.sixteen_bit) {
        (3, _) => "PNG RGB 8-bit",
        (_, true) => "PNG grayscale 16-bit",
        _ => "PNG grayscale 8-bit",
    };
    let mut m = json!({
        "prefix": spec.prefix,
        "format": format,
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
        m["decode"] = json!("R = class a, G = class b, B = weight of a (code / 255)");
    }
    if let Some(meaning) = spec.meaning {
        m["meaning"] = json!(meaning);
    }
    if spec.name == WATER_SURFACE.name {
        m["extended_texels"] = json!(super::water::REACH);
        m["wrap_m"] = json!(WATER_WRAP_M);
        m["decode"] = json!(format!(
            "wrapped: s0 = offset + code · scale; surface_m = s0 + n · wrap_m with n = round((ground_m − s0) / wrap_m), ground_m the decoded height of the same texel. |surface − ground| < {WATER_UNWRAP_LIMIT_M} m on every level-0 texel and every extension texel (farther extensions are dropped); coarse mixed shore texels can exceed it (validation figure coarse_surface_far_from_ground), so take sea, lake and ice surfaces from water_bodies[].level_m"
        ));
    }
    m
}
