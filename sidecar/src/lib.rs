//! telegram-seoyu sidecar library
//!
//! Owns the local message mirror, Korean-aware search index, and wiki
//! pipeline. Consumed by the Swift shell via the IPC binary at
//! `src/bin/main.rs`, which serves requests over a Unix-domain
//! socket.

pub mod error;
pub mod ipc;
pub mod logging;
pub mod search;
pub mod security;
pub mod store;
pub mod uniffi_api;
pub mod wiki;

// Emit the UniFFI scaffolding. Must come after the `uniffi_api` module
// is declared so the proc-macro sees the exported types.
uniffi::setup_scaffolding!("seoyu");
