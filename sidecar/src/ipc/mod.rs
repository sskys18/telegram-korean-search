//! IPC layer for the Swift shell.
//!
//! Wire format is length-prefixed JSON on a Unix domain socket. Each
//! frame is `u32` big-endian byte length followed by that many UTF-8
//! JSON bytes. Requests and responses follow the shape in
//! [`protocol`]. The socket path is derived at startup and printed to
//! stdout so the Swift parent can read it off the sidecar's first
//! stdout line.
//!
//! The server in [`server`] accepts connections and dispatches
//! individual messages through [`handlers::dispatch`].

pub mod codec;
pub mod handlers;
pub mod protocol;
pub mod server;

pub use server::{default_socket_path, serve, SidecarServer};
