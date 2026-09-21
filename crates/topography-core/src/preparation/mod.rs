//! Regional quality assessment and preparation eligibility.

mod quality;
mod region;

pub(crate) use quality::Counts;
pub use quality::{assess, check, draft};
pub(crate) use region::verify_files;
pub use region::{build, verify};
