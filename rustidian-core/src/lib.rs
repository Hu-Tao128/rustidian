//! rustidian-core — pure logic crate.
//!
//! All disk operations, Markdown processing, link indexing, and search live here.
//! This crate never imports `slint` or any UI crate.

pub mod config;
pub mod error;
pub mod links;
pub mod markdown;
pub mod search;
pub mod vault;

#[cfg(feature = "graph")]
pub mod graph;

pub use error::CoreError;
