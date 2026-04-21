//! telegram-seoyu sidecar binary
//!
//! Stub entry point. Later phases wire this up to a Unix-socket
//! JSON-RPC server that the Swift shell talks to. For now it just
//! starts logging and opens the store so the crate compiles
//! end-to-end.

use seoyu::{logging, store};

fn main() {
    let log_dir = store::app_data_dir();
    if let Err(e) = logging::init(&log_dir) {
        eprintln!("failed to initialize logging: {e}");
    }

    log::info!(
        "telegram-seoyu sidecar v{} starting",
        env!("CARGO_PKG_VERSION")
    );

    let db_path = store::default_db_path();
    match store::Store::open(&db_path) {
        Ok(_) => log::info!("store opened at {}", db_path.display()),
        Err(e) => log::error!("failed to open store: {e}"),
    }

    log::info!("IPC server not yet implemented; exiting");
}
