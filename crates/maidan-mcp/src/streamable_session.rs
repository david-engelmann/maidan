//! Active `POST /mcp/streamable` sessions (`Mcp-Session-Id`, Cluster 35; TTL Cluster 60).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

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
        self.sessions.lock().await.insert(
            id,
            SessionEntry {
                tx,
                last_touch: Instant::now(),
                next_event_id: 0,
                log: VecDeque::new(),
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
    async fn expired_session_is_not_open() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_millis(1));
        let _rx = reg.open("sess".to_string()).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!reg.is_open("sess").await);
    }
}
