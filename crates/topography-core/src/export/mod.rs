//! Heightmap texture tiles, height encoding, masks, and export validation.

mod heightmap;
mod view;
pub use heightmap::{Layout, heightmap};
pub use view::view;

/// Short layer name used in tile file names.
fn tile_prefix(layer: &str) -> &str {
    match layer {
        "vegetation" => "veg",
        "snow-ice" => "snow",
        "land-cover" => "landcover",
        other => other,
    }
}

/// Path of a tile below the export root: `{layer}/L{level}/{prefix}_L{level}_{tx}_{ty}.png`,
/// e.g. `vegetation/L0/veg_L0_0_0.png`.
fn tile_path(layer: &str, level: impl std::fmt::Display, tx: usize, ty: usize) -> String {
    format!(
        "{layer}/L{level}/{}_L{level}_{tx}_{ty}.png",
        tile_prefix(layer)
    )
}
