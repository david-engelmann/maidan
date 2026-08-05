//! Active `POST /mcp/streamable` sessions (`Mcp-Session-Id`, Cluster 35; TTL Cluster 60).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::{broadcast, oneshot, Mutex};

/// Per-session SSE buffer bound (Cluster 129). A slow client that fills this is
/// disconnected (push returns false) instead of growing server memory without
/// limit — the channel was previously unbounded.
const SESSION_BUFFER: usize = 256;

/// How many recently-sent events to retain per session for `Last-Event-ID`
/// replay on reconnect (Cluster 147). Bounded like the live buffer.
const SESSION_LOG_CAP: usize = 256;

struct SessionEntry {
    tx: tokio::sync::mpsc::Sender<(u64, String)>,
    last_touch: Instant,
    /// Monotonic SSE event id assigned to the next pushed message.
    next_event_id: u64,
    /// Recent `(event_id, payload)` frames, for `Last-Event-ID` replay.
    log: VecDeque<(u64, String)>,
    /// The client's `capabilities` object from `initialize` (Cluster 148) —
    /// gates server→client requests (sampling / roots / elicitation).
    client_capabilities: Value,
    /// Next id for a server→client request, and the reply slots awaiting the
    /// client's response, keyed by that id.
    next_request_id: i64,
    pending: HashMap<i64, oneshot::Sender<Value>>,
    /// Server→client requests are delivered on the spec-canonical
    /// `GET /mcp/streamable` stream (Cluster 154). This broadcast fans a pushed
    /// request out to every open GET leg for the session; the POST leg (which
    /// consumes `tx`) carries only the request/response + notifications.
    client_req_tx: broadcast::Sender<String>,
}

#[derive(Clone)]
pub struct StreamableSessionRegistry {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    ttl: Duration,
}

impl Default for StreamableSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamableSessionRegistry {
    pub fn new() -> Self {
        Self::with_ttl(streamable_session_ttl())
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    async fn prune_expired(&self) {
        let ttl = self.ttl;
        let mut guard = self.sessions.lock().await;
        guard.retain(|_, entry| entry.last_touch.elapsed() < ttl);
    }

    /// Open a new session and return the SSE consumer side. Each item is the
    /// `(event_id, payload)` pair; the transport renders the id as the SSE
    /// `id:` field so a client can resume with `Last-Event-ID`.
    pub async fn open(&self, id: String) -> tokio::sync::mpsc::Receiver<(u64, String)> {
        self.prune_expired().await;
        let (tx, rx) = tokio::sync::mpsc::channel(SESSION_BUFFER);
        let (client_req_tx, _) = broadcast::channel(SESSION_BUFFER);
        self.sessions.lock().await.insert(
            id,
            SessionEntry {
                tx,
                last_touch: Instant::now(),
                next_event_id: 0,
                log: VecDeque::new(),
                client_capabilities: Value::Null,
                next_request_id: 0,
                pending: HashMap::new(),
                client_req_tx,
            },
        );
        rx
    }

    pub async fn touch(&self, id: &str) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.last_touch = Instant::now();
        }
    }

    pub async fn is_open(&self, id: &str) -> bool {
        self.prune_expired().await;
        self.sessions.lock().await.contains_key(id)
    }

    pub async fn push(&self, id: &str, data: String) -> bool {
        self.prune_expired().await;
        let mut guard = self.sessions.lock().await;
        let Some(entry) = guard.get_mut(id) else {
            return false;
        };
        // Assign the event id and record the frame for replay before delivering,
        // so a client that reconnects with `Last-Event-ID` can recover it even
        // if the live consumer had already dropped.
        let event_id = entry.next_event_id;
        entry.next_event_id += 1;
        entry.log.push_back((event_id, data.clone()));
        while entry.log.len() > SESSION_LOG_CAP {
            entry.log.pop_front();
        }
        // Non-blocking: we hold the registry lock, so never await on capacity.
        // A full buffer (slow client) or a closed receiver both fail the live
        // delivery; the caller stops the stream, but the session (and its log)
        // stays open for reconnect until TTL/DELETE.
        match entry.tx.try_send((event_id, data)) {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    session = id,
                    "mcp streamable session buffer full; dropping client"
                );
                false
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Frames retained for this session with an id greater than `after`, in
    /// order — the `Last-Event-ID` replay set (Cluster 147). Empty for an
    /// unknown session or when nothing newer was retained.
    pub async fn replay_after(&self, id: &str, after: u64) -> Vec<(u64, String)> {
        let mut guard = self.sessions.lock().await;
        let Some(entry) = guard.get_mut(id) else {
            return Vec::new();
        };
        entry.last_touch = Instant::now();
        entry
            .log
            .iter()
            .filter(|(event_id, _)| *event_id > after)
            .cloned()
            .collect()
    }

    /// Record the client's declared `capabilities` (from `initialize`).
    pub async fn set_client_capabilities(&self, id: &str, capabilities: Value) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.client_capabilities = capabilities;
        }
    }

    /// Whether the session's client declared a top-level capability key
    /// (e.g. `sampling`, `roots`, `elicitation`).
    pub async fn client_supports(&self, id: &str, capability: &str) -> bool {
        self.sessions
            .lock()
            .await
            .get(id)
            .is_some_and(|e| e.client_capabilities.get(capability).is_some())
    }

    /// Allocate the next server→client request id and a reply slot; returns the
    /// id + the receiver that resolves when the client's response arrives.
    /// `None` if the session is gone.
    pub async fn register_client_request(
        &self,
        id: &str,
    ) -> Option<(i64, oneshot::Receiver<Value>)> {
        let mut guard = self.sessions.lock().await;
        let entry = guard.get_mut(id)?;
        let request_id = entry.next_request_id;
        entry.next_request_id += 1;
        let (tx, rx) = oneshot::channel();
        entry.pending.insert(request_id, tx);
        Some((request_id, rx))
    }

    /// Deliver a server→client request payload on the session's GET-stream
    /// broadcast. Returns whether it reached at least one open GET leg — a
    /// `false` means no client is listening on the spec-canonical stream, so
    /// the caller should fail fast rather than await a response that can't come.
    pub async fn push_client_request(&self, id: &str, data: String) -> bool {
        self.prune_expired().await;
        let mut guard = self.sessions.lock().await;
        let Some(entry) = guard.get_mut(id) else {
            return false;
        };
        entry.last_touch = Instant::now();
        entry.client_req_tx.send(data).is_ok()
    }

    /// Subscribe a `GET /mcp/streamable` leg to this session's server→client
    /// requests. `None` if the session is gone.
    pub async fn subscribe_client_requests(&self, id: &str) -> Option<broadcast::Receiver<String>> {
        let mut guard = self.sessions.lock().await;
        let entry = guard.get_mut(id)?;
        entry.last_touch = Instant::now();
        Some(entry.client_req_tx.subscribe())
    }

    /// Route the client's response (by request id) to the awaiting caller.
    /// Returns whether a pending request matched.
    pub async fn resolve_client_response(
        &self,
        id: &str,
        request_id: i64,
        response: Value,
    ) -> bool {
        let mut guard = self.sessions.lock().await;
        let Some(entry) = guard.get_mut(id) else {
            return false;
        };
        match entry.pending.remove(&request_id) {
            Some(tx) => tx.send(response).is_ok(),
            None => false,
        }
    }

    /// Drop a pending request slot (timeout / undeliverable).
    pub async fn cancel_client_request(&self, id: &str, request_id: i64) {
        if let Some(entry) = self.sessions.lock().await.get_mut(id) {
            entry.pending.remove(&request_id);
        }
    }

    pub async fn close(&self, id: &str) {
        self.sessions.lock().await.remove(id);
    }
}

pub fn streamable_session_ttl() -> Duration {
    std::env::var("MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(3600))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_push_and_close() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let mut rx = reg.open("sess".to_string()).await;
        assert!(reg.is_open("sess").await);
        assert!(reg.push("sess", "payload".into()).await);
        assert_eq!(rx.recv().await.unwrap(), (0, "payload".to_string()));
        reg.close("sess").await;
        assert!(!reg.is_open("sess").await);
        assert!(!reg.push("sess", "x".into()).await);
    }

    #[tokio::test]
    async fn replay_after_returns_only_newer_frames() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let _rx = reg.open("sess".to_string()).await;
        for i in 0..3 {
            assert!(reg.push("sess", format!("m{i}")).await);
        }
        // Frames are id'd 0,1,2; replaying after id 0 yields 1 and 2 in order.
        let replay = reg.replay_after("sess", 0).await;
        assert_eq!(replay, vec![(1, "m1".to_string()), (2, "m2".to_string())]);
        // Replaying after the latest id yields nothing; unknown session yields nothing.
        assert!(reg.replay_after("sess", 2).await.is_empty());
        assert!(reg.replay_after("missing", 0).await.is_empty());
    }

    #[tokio::test]
    async fn full_session_buffer_fails_push_without_blocking() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let _rx = reg.open("sess".to_string()).await; // never drained
                                                      // Fill the bounded buffer.
        for _ in 0..SESSION_BUFFER {
            assert!(reg.push("sess", "x".into()).await);
        }
        // The next push finds the buffer full and fails (non-blocking) rather
        // than growing memory or hanging.
        assert!(!reg.push("sess", "overflow".into()).await);
    }

    #[tokio::test]
    async fn client_capabilities_and_pending_request_correlation() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let _rx = reg.open("sess".to_string()).await;
        reg.set_client_capabilities("sess", serde_json::json!({"sampling": {}}))
            .await;
        assert!(reg.client_supports("sess", "sampling").await);
        assert!(!reg.client_supports("sess", "roots").await);
        assert!(!reg.client_supports("missing", "sampling").await);

        // Register a server→client request, then resolve it by id.
        let (req_id, rx) = reg.register_client_request("sess").await.unwrap();
        assert_eq!(req_id, 0);
        assert!(
            reg.resolve_client_response("sess", req_id, serde_json::json!({"result": 1}))
                .await
        );
        assert_eq!(rx.await.unwrap(), serde_json::json!({"result": 1}));
        // A second resolve of the same id finds nothing pending.
        assert!(
            !reg.resolve_client_response("sess", req_id, serde_json::json!({}))
                .await
        );
    }

    #[tokio::test]
    async fn client_requests_reach_a_get_stream_subscriber() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let _mpsc = reg.open("sess".to_string()).await;

        // No GET leg subscribed yet → a request is undeliverable (fail fast).
        assert!(!reg.push_client_request("sess", "req0".into()).await);

        // A subscribed GET leg receives requests; the POST-leg mpsc is separate.
        let mut get_rx = reg.subscribe_client_requests("sess").await.unwrap();
        assert!(reg.push_client_request("sess", "req1".into()).await);
        assert_eq!(get_rx.recv().await.unwrap(), "req1");

        // Unknown session: no subscriber, no delivery.
        assert!(reg.subscribe_client_requests("missing").await.is_none());
        assert!(!reg.push_client_request("missing", "x".into()).await);
    }

    #[tokio::test]
    async fn expired_session_is_not_open() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_millis(1));
        let _rx = reg.open("sess".to_string()).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!reg.is_open("sess").await);
    }
}
