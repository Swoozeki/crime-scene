//! Behavioral code analyses over the ingested history.

pub mod coupling;
pub mod dataset;
pub mod detail;
pub mod diff;
pub mod engine;
pub mod findings;
pub mod hotspots;
mod identity;
pub mod overview;
pub mod scan;
pub mod scope;
pub mod social;
pub mod trends;
pub mod xray;

pub use dataset::Dataset;
pub use engine::Engine;
pub use scope::{Level, Scope};

#[cfg(test)]
mod tests;
