//! Base terrain, real-patch composition, ownership, reversible edits, and regional rebuilds.

pub mod hydrology;
mod input;
mod layout;
mod preview;
mod sketch;
pub mod water;
pub use input::build_input;
pub use layout::build_layout;
pub use preview::build_preview;
pub use sketch::build_sketch;
