//! WebSocket subscribe + the `wait_for_*` helpers (over [`tungstenite`]).

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

use crate::{Client, MaidanError, Result};

/// A live subscription handle. Dropping it (or calling [`Subscription::close`])
/// stops the reader thread and closes the socket.
pub struct Subscription {
    closed: Arc<AtomicBool>,
    shutdown: Option<TcpStream>,
    handle: Option<JoinHandle<()>>,
}

impl Subscription {
    /// Stop the subscription and close the socket.
    pub fn close(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(s) = self.shutdown.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Client {
    /// Subscribe to the event stream over WebSocket. `filter` follows
    /// `contracts/ws-subscribe-filter.schema.json` (set `workspace_id` for replay).
    /// Control frames (subscribe_ack, schema_version, replay_*) are skipped; each
    /// domain event is passed to `on_event`. Unknown kinds are still delivered.
    pub fn subscribe<F>(&self, filter: Value, on_event: F) -> Result<Subscription>
    where
        F: Fn(Value) + Send + 'static,
    {
        let ws_url = ws_url(&self.base_url);
        let (mut socket, _resp) =
            tungstenite::connect(&ws_url).map_err(|e| MaidanError::transport(e.to_string()))?;
        let shutdown = clone_tcp(socket.get_ref());

        let frame = json!({ "filter": filter, "token": self.token });
        socket
            .send(Message::Text(frame.to_string()))
            .map_err(|e| MaidanError::transport(e.to_string()))?;

        let closed = Arc::new(AtomicBool::new(false));
        let closed_reader = closed.clone();
        let handle = std::thread::spawn(move || loop {
            if closed_reader.load(Ordering::SeqCst) {
                break;
            }
            match socket.read() {
                Ok(Message::Text(t)) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&t) {
                        if v.get("type").is_some() {
                            continue; // control frame
                        }
                        if v.get("kind").and_then(|k| k.as_str()).is_some() {
                            on_event(v);
                        }
                    }
                }
                Ok(Message::Ping(p)) => {
                    let _ = socket.send(Message::Pong(p));
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        });

        Ok(Subscription {
            closed,
            shutdown,
            handle: Some(handle),
        })
    }

    fn wait_for_kind(
        &self,
        mut filter: Value,
        kind: &str,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        if !filter.is_object() {
            filter = json!({});
        }
        filter["kinds"] = json!([kind]);
        let (tx, rx) = mpsc::channel();
        let sub = self.subscribe(filter, move |e| {
            let _ = tx.send(e);
        })?;
        let out = rx.recv_timeout(timeout).ok();
        sub.close();
        Ok(out)
    }

    /// Block until the thread's result is set, or `timeout` (`Ok(None)`).
    pub fn wait_for_result(
        &self,
        thread_id: &str,
        workspace_id: &str,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        self.wait_for_kind(
            json!({ "workspace_id": workspace_id, "thread_id": thread_id }),
            "thread_result_set",
            timeout,
        )
    }

    /// Block until the member is mentioned, or `timeout` (`Ok(None)`).
    pub fn wait_for_mention(
        &self,
        member_id: &str,
        workspace_id: &str,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        self.wait_for_kind(
            json!({ "workspace_id": workspace_id, "member_id": member_id }),
            "mention_recorded",
            timeout,
        )
    }

    /// Block until a task becomes claimable, or `timeout` (`Ok(None)`).
    /// `channel_id == None` scopes to the whole workspace.
    pub fn wait_for_ready(
        &self,
        workspace_id: &str,
        channel_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        let mut f = json!({ "workspace_id": workspace_id });
        if let Some(c) = channel_id {
            f["channel_id"] = json!(c);
        }
        self.wait_for_kind(f, "thread_ready", timeout)
    }
}

fn ws_url(base_url: &str) -> String {
    let scheme_swapped = if let Some(rest) = base_url.strip_prefix("http") {
        format!("ws{rest}")
    } else {
        base_url.to_string()
    };
    format!("{scheme_swapped}/ws/subscribe")
}

/// Grab a clonable `TcpStream` so `close()` can unblock a parked `read()` via
/// shutdown. Returns `None` for stream kinds we can't clone (close then relies on
/// the flag + the next inbound frame / error).
fn clone_tcp(stream: &MaybeTlsStream<TcpStream>) -> Option<TcpStream> {
    match stream {
        MaybeTlsStream::Plain(s) => s.try_clone().ok(),
        MaybeTlsStream::Rustls(s) => s.get_ref().try_clone().ok(),
        _ => None,
    }
}
