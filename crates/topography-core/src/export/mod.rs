//! Heightmap and game layer texture tiles, height encoding, and export validation.

mod checks;
mod heightmap;
mod layers;
mod legends;
mod pyramid;
#[cfg(test)]
pub(crate) mod testworld;
mod tiles;
mod view;
mod water;
pub use heightmap::{Layout, heightmap};
pub use view::view;

/// Short layer name used in tile file names.
fn tile_prefix(layer: &str) -> &str {
    match layer {
        "land-cover" => "landcover",
        other => other,
    }
}

/// Path of a tile below the export root: `{layer}/L{level}/{prefix}_L{level}_{tx}_{ty}.png`,
/// e.g. `land-cover/L1/landcover_L1_0_0.png`.
fn tile_path(layer: &str, level: impl std::fmt::Display, tx: usize, ty: usize) -> String {
    format!(
        "{layer}/L{level}/{}_L{level}_{tx}_{ty}.png",
        tile_prefix(layer)
    )
}
