//! Regional quality assessment and preparation eligibility.

/// Minimum valid share of ground cells, per mille. Missing cells stay masked in
/// training, so partial tiles remain useful game terrain (owner, 2026-09-26).
pub(crate) const MIN_VALID_GROUND_PERMILLE: u64 = 500;

pub mod automatic;
mod quality;
mod region;

pub(crate) use quality::Counts;
pub use quality::{assess, check, draft};
pub(crate) use region::verify_files;
pub use region::{build, verify};

/// Compares GDAL geotransforms allowing for last-digit float text round-trip differences.
pub(crate) fn same_transform(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    let (Some(a), Some(b)) = (a.as_array(), b.as_array()) else {
        return false;
    };
    a.len() == 6
        && b.len() == 6
        && a.iter()
            .zip(b)
            .all(|(x, y)| match (x.as_f64(), y.as_f64()) {
                (Some(x), Some(y)) => (x - y).abs() <= 1e-12 * x.abs().max(y.abs()).max(1.0),
                _ => false,
            })
}
