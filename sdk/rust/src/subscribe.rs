//! WebSocket subscribe + the `wait_for_*` / [`Client::follow`] helpers (over [`tungstenite`]).

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

/// Projector shape + cursor for [`Client::follow`]: HTTP backfill, then WS cutover.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Follow {
    pub workspace_id: String,
    pub channel_id: Option<String>,
    pub thread_id: Option<String>,
    pub types: Vec<String>,
    pub consumer_id: Option<String>,
    pub after_id: i64,
    /// HTTP page size. `0` uses the server default (100).
    pub page_limit: i64,
}

impl Follow {
    pub fn workspace(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            ..Self::default()
        }
    }
}

impl Client {
    /// Subscribe to the event stream over WebSocket. `filter` follows
    /// `contracts/ws-subscribe-filter.schema.json` (set `workspace_id` for replay).
    /// Benign control frames (`subscribe_ack`, `schema_version`, `replay_*`) are
    /// skipped. `type: cursor_too_old` is delivered (then the reader stops) —
    /// never treated as a silent skip. Unknown kinds are still delivered.
    pub fn subscribe<F>(&self, filter: Value, on_event: F) -> Result<Subscription>
    where
        F: Fn(Value) + Send + 'static,
    {
        self.subscribe_from(filter, 0, None, on_event)
    }

    /// Subscribe with an explicit `after_id` / optional durable `consumer_id`
    /// on the subscribe frame (siblings of `filter`, not inside it).
    pub fn subscribe_from<F>(
        &self,
        filter: Value,
        after_id: i64,
        consumer_id: Option<&str>,
        on_event: F,
    ) -> Result<Subscription>
    where
        F: Fn(Value) + Send + 'static,
    {
        let ws_url = ws_url(&self.base_url);
        let (mut socket, _resp) =
            tungstenite::connect(&ws_url).map_err(|e| MaidanError::transport(e.to_string()))?;
        let shutdown = clone_tcp(socket.get_ref());

        let mut frame = json!({ "filter": filter, "token": self.token });
        if after_id > 0 {
            frame["after_id"] = json!(after_id);
        }
        if let Some(id) = consumer_id {
            frame["consumer_id"] = json!(id);
        }
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
                        if is_cursor_too_old_frame(&v) {
                            on_event(v);
                            break;
                        }
                        if is_benign_control(&v) {
                            continue;
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

    /// HTTP backfill `GET /workspaces/{id}/events` for the projector shape, then
    /// cut over to WebSocket at the last seen id. A 409 `must_refetch` is
    /// returned as [`MaidanError::is_cursor_too_old`] — never clamped.
    ///
    /// Each backfill row is normalized to the live frame shape (`log_id` +
    /// flattened payload) before `on_event`.
    pub fn follow<F>(&self, spec: &Follow, on_event: F) -> Result<Subscription>
    where
        F: Fn(Value) + Send + 'static,
    {
        let limit = if spec.page_limit > 0 {
            spec.page_limit
        } else {
            100
        };
        let mut after = spec.after_id;
        loop {
            let query = follow_query(spec, after, limit);
            let refs: Vec<(&str, &str)> = query
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let page = self.list_events(&spec.workspace_id, &refs)?;
            let rows = page
                .as_array()
                .ok_or_else(|| MaidanError::transport("list_events: expected a JSON array"))?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                if let Some(id) = stored_id(row) {
                    after = after.max(id);
                }
                on_event(normalize_stored(row));
            }
            if (rows.len() as i64) < limit {
                break;
            }
        }

        self.subscribe_from(
            follow_filter(spec),
            after,
            spec.consumer_id.as_deref(),
            on_event,
        )
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

fn is_benign_control(v: &Value) -> bool {
    matches!(
        v.get("type").and_then(Value::as_str),
        Some("subscribe_ack" | "schema_version" | "replay_hint" | "replay_truncated")
    )
}

fn is_cursor_too_old_frame(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("cursor_too_old")
}

fn stored_id(row: &Value) -> Option<i64> {
    row.get("id")
        .and_then(Value::as_i64)
        .or_else(|| row.get("log_id").and_then(Value::as_i64))
}

/// HTTP `StoredEvent` (`id` + nested `payload`) → live bus shape (`log_id` + flat event).
fn normalize_stored(row: &Value) -> Value {
    let mut out = match row.get("payload") {
        Some(p) if p.is_object() => p.clone(),
        _ => row.clone(),
    };
    if let Some(obj) = out.as_object_mut() {
        if let Some(id) = stored_id(row) {
            obj.insert("log_id".into(), json!(id));
        }
        for key in ["kind", "workspace_id", "channel_id", "thread_id"] {
            if !obj.contains_key(key) {
                if let Some(v) = row.get(key) {
                    obj.insert(key.to_string(), v.clone());
                }
            }
        }
    }
    out
}

fn follow_query(spec: &Follow, after_id: i64, limit: i64) -> Vec<(String, String)> {
    let mut q = vec![
        ("after_id".into(), after_id.to_string()),
        ("limit".into(), limit.to_string()),
    ];
    if let Some(ch) = &spec.channel_id {
        q.push(("channel_id".into(), ch.clone()));
    }
    if let Some(th) = &spec.thread_id {
        q.push(("thread_id".into(), th.clone()));
    }
    if !spec.types.is_empty() {
        q.push(("types".into(), spec.types.join(",")));
    }
    if let Some(id) = &spec.consumer_id {
        q.push(("consumer_id".into(), id.clone()));
    }
    q
}

fn follow_filter(spec: &Follow) -> Value {
    let mut f = json!({ "workspace_id": spec.workspace_id });
    if let Some(ch) = &spec.channel_id {
        f["channel_id"] = json!(ch);
    }
    if let Some(th) = &spec.thread_id {
        f["thread_id"] = json!(th);
    }
    if !spec.types.is_empty() {
        f["kinds"] = json!(spec.types);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benign_controls_are_skipped_but_cursor_too_old_is_not() {
        assert!(is_benign_control(&json!({"type": "subscribe_ack"})));
        assert!(is_benign_control(&json!({"type": "schema_version"})));
        assert!(is_benign_control(&json!({"type": "replay_hint"})));
        assert!(is_benign_control(&json!({"type": "replay_truncated"})));
        assert!(!is_benign_control(
            &json!({"type": "cursor_too_old", "must_refetch": true})
        ));
        assert!(is_cursor_too_old_frame(
            &json!({"type": "cursor_too_old", "must_refetch": true, "after_id": 1})
        ));
        assert!(!is_cursor_too_old_frame(&json!({"kind": "message_posted"})));
    }

    #[test]
    fn normalize_stored_promotes_id_and_flattens_payload() {
        let row = json!({
            "id": 42,
            "kind": "message_posted",
            "workspace_id": "ws",
            "payload": { "kind": "message_posted", "body": "hi", "workspace_id": "ws" }
        });
        let live = normalize_stored(&row);
        assert_eq!(live["log_id"], 42);
        assert_eq!(live["kind"], "message_posted");
        assert_eq!(live["body"], "hi");
    }

    #[test]
    fn follow_query_includes_shape_and_cursor() {
        let spec = Follow {
            workspace_id: "ws".into(),
            channel_id: Some("ch".into()),
            thread_id: None,
            types: vec!["message_posted".into(), "thread_ready".into()],
            consumer_id: Some("proj-1".into()),
            after_id: 7,
            page_limit: 50,
        };
        let q = follow_query(&spec, 9, 50);
        assert!(q.contains(&("after_id".into(), "9".into())));
        assert!(q.contains(&("channel_id".into(), "ch".into())));
        assert!(q.contains(&("types".into(), "message_posted,thread_ready".into())));
        assert!(q.contains(&("consumer_id".into(), "proj-1".into())));
        let f = follow_filter(&spec);
        assert_eq!(f["workspace_id"], "ws");
        assert_eq!(f["kinds"], json!(["message_posted", "thread_ready"]));
    }
}
