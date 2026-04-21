//! telegram-seoyu sidecar library
//!
//! Owns the local message mirror, Korean-aware search index, and wiki
//! pipeline. Consumed by the Swift shell via the IPC binary at
//! `src/bin/main.rs`.

pub mod error;
pub mod logging;
pub mod search;
pub mod security;
pub mod store;
pub mod wiki;
