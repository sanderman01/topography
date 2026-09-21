//! Regional quality assessment and preparation eligibility.

mod quality;

pub(crate) use quality::Counts;
pub use quality::{assess, check, draft};
