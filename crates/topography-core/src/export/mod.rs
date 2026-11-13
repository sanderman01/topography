//! Heightmap and game layer texture tiles, height encoding, and export validation.

mod bc;
mod checks;
mod description;
mod heightmap;
mod ktx2;
mod layers;
mod legends;
mod normals;
mod pyramid;
#[cfg(test)]
pub(crate) mod testworld;
mod tiles;
mod view;
mod water;
pub use heightmap::{Layout, heightmap};
pub use normals::NormalOptions;
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
    tile_file(layer, level, tx, ty, "png")
}

/// Like `tile_path` with another extension, e.g. `normal/L2/normal_L2_0_0.ktx2`.
fn tile_file(
    layer: &str,
    level: impl std::fmt::Display,
    tx: usize,
    ty: usize,
    ext: &str,
) -> String {
    format!(
        "{layer}/L{level}/{}_L{level}_{tx}_{ty}.{ext}",
        tile_prefix(layer)
    )
}
