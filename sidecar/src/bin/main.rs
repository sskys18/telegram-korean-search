//! telegram-seoyu sidecar binary.
//!
//! Opens the sqlite store, binds a Unix-domain socket, prints the
//! socket path to stdout (the Swift shell reads it from the first
//! line of child stdout), and then serves IPC requests until the
//! connected client asks for shutdown or the process is killed.

use std::process::ExitCode;

use seoyu::ipc::{default_socket_path, handlers::SidecarState, serve};
use seoyu::{logging, store};

fn main() -> ExitCode {
    let log_dir = store::app_data_dir();
    if let Err(e) = logging::init(&log_dir) {
        eprintln!("failed to initialize logging: {e}");
    }

    log::info!(
        "telegram-seoyu sidecar v{} starting",
        env!("CARGO_PKG_VERSION")
    );

    let db_path = store::default_db_path();
    let store_handle = match store::Store::open(&db_path) {
        Ok(s) => {
            log::info!("store opened at {}", db_path.display());
            s
        }
        Err(e) => {
            log::error!("failed to open store: {e}");
            return ExitCode::from(1);
        }
    };

    let state = SidecarState::new(store_handle);
    let socket_path = default_socket_path();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("failed to build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    if let Err(e) = runtime.block_on(serve(state, socket_path)) {
        log::error!("sidecar server exited with error: {e}");
        return ExitCode::from(1);
    }

    log::info!("sidecar exiting normally");
    ExitCode::SUCCESS
}
