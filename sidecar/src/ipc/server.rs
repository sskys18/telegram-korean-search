//! Unix-domain-socket server for the Swift shell.
//!
//! The sidecar's `main` decides the socket path, prints it to stdout,
//! then calls [`serve`]. Each accepted connection runs in its own
//! tokio task and processes one request at a time in the order the
//! client sent them; there is no per-connection pipelining because
//! the Swift client is naturally sequential (keystrokes, scrolls).
//! Future server-initiated events are also written through the same
//! connection and multiplexed in the handler task.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::ipc::codec::{read_frame, write_frame};
use crate::ipc::handlers::{dispatch_request, handle_notification, Dispatch, SidecarState};
use crate::ipc::protocol::{Event, Incoming, OutgoingFrame, Response, RpcError, ServerEvent};

/// Default socket path. Falls back to `/tmp` when `$TMPDIR` is not
/// set (macOS always sets it; Linux CI sometimes does not). Keeps
/// the file name short so the 104-byte path limit on UDS is never
/// an issue in practice.
pub fn default_socket_path() -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("telegram-seoyu-sidecar.sock")
}

/// Handle used by the server owner to push events to the currently
/// connected shell. Dropping the handle simply stops pushes; the
/// accept loop keeps running until you await [`SidecarServer::run`]
/// finishes.
#[derive(Clone)]
pub struct EventSender {
    tx: mpsc::UnboundedSender<ServerEvent>,
}

impl EventSender {
    pub fn send(&self, event: ServerEvent) {
        let _ = self.tx.send(event);
    }
}

pub struct SidecarServer {
    listener: UnixListener,
    state: SidecarState,
    event_rx: mpsc::UnboundedReceiver<ServerEvent>,
    socket_path: PathBuf,
}

impl SidecarServer {
    /// Bind to `path`, removing any stale socket file first.
    pub fn bind(path: impl AsRef<Path>, state: SidecarState) -> io::Result<(Self, EventSender)> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(&path)?;
        let (tx, rx) = mpsc::unbounded_channel();
        Ok((
            Self {
                listener,
                state,
                event_rx: rx,
                socket_path: path,
            },
            EventSender { tx },
        ))
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Serve one connection at a time. When a client disconnects, the
    /// next incoming connection is accepted. Exits only when the
    /// listener returns a fatal error or a handler returns
    /// [`Dispatch::Shutdown`].
    pub async fn run(mut self) -> io::Result<()> {
        loop {
            tokio::select! {
                accept = self.listener.accept() => {
                    match accept {
                        Ok((stream, _addr)) => {
                            log::info!("sidecar: client connected");
                            let keep_going = handle_conn(
                                stream,
                                self.state.clone(),
                                &mut self.event_rx,
                            ).await?;
                            log::info!("sidecar: client disconnected");
                            if !keep_going {
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            log::error!("sidecar: accept error: {e}");
                            return Err(e);
                        }
                    }
                }
                // When no one is connected, drain any queued events so
                // the channel cannot grow unboundedly.
                maybe_event = self.event_rx.recv() => {
                    if let Some(event) = maybe_event {
                        log::debug!("sidecar: dropping event (no client connected): {event:?}");
                    }
                }
            }
        }
    }
}

/// Process one client connection. Returns `Ok(false)` if a request
/// asked the server to shut down.
async fn handle_conn(
    stream: UnixStream,
    state: SidecarState,
    events: &mut mpsc::UnboundedReceiver<ServerEvent>,
) -> io::Result<bool> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);

    loop {
        tokio::select! {
            frame = read_frame(&mut reader) => {
                match frame? {
                    None => return Ok(true),
                    Some(bytes) => {
                        match serde_json::from_slice::<Incoming>(&bytes) {
                            Ok(Incoming::Request(req)) => {
                                let id = req.id;
                                match dispatch_request(&state, req) {
                                    Dispatch::Reply(resp) => {
                                        send_response(&mut writer, resp).await?;
                                    }
                                    Dispatch::Silent => {}
                                    Dispatch::Shutdown => {
                                        let resp = Response {
                                            id,
                                            outcome: crate::ipc::protocol::Outcome::Ok {
                                                result: crate::ipc::protocol::ResponsePayload::ShutdownAck,
                                            },
                                        };
                                        send_response(&mut writer, resp).await?;
                                        writer.shutdown().await.ok();
                                        return Ok(false);
                                    }
                                }
                            }
                            Ok(Incoming::Notification(note)) => {
                                handle_notification(&state, note);
                            }
                            Err(e) => {
                                log::warn!("sidecar: malformed frame: {e}");
                                let err_body = serde_json::to_vec(&OutgoingFrame::Response(Response {
                                    id: 0,
                                    outcome: crate::ipc::protocol::Outcome::Err {
                                        error: RpcError {
                                            code: RpcError::INVALID_PARAMS,
                                            message: e.to_string(),
                                        },
                                    },
                                }))
                                .expect("serde_json must serialize RpcError");
                                write_frame(&mut writer, &err_body).await?;
                            }
                        }
                    }
                }
            }
            maybe_event = events.recv() => {
                match maybe_event {
                    Some(event) => {
                        let body = serde_json::to_vec(&OutgoingFrame::Event(Event { body: event }))
                            .expect("ServerEvent must serialize");
                        write_frame(&mut writer, &body).await?;
                    }
                    None => {
                        // Channel closed: the server is shutting down.
                        return Ok(false);
                    }
                }
            }
        }
    }
}

async fn send_response<W>(writer: &mut W, resp: Response) -> io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(&OutgoingFrame::Response(resp)).expect("Response must serialize");
    write_frame(writer, &body).await
}

/// Convenience entry point: bind, announce path on stdout, and serve.
pub async fn serve(state: SidecarState, path: PathBuf) -> io::Result<()> {
    let (server, _events) = SidecarServer::bind(&path, state)?;
    println!("{}", server.socket_path().display());
    server.run().await
}
