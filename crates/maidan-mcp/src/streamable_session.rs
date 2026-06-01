//! Active `POST /mcp/streamable` sessions (`Mcp-Session-Id`, Cluster 35; TTL Cluster 60).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

struct SessionEntry {
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    last_touch: Instant,
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

    /// Open a new session and return the SSE consumer side.
    pub async fn open(&self, id: String) -> tokio::sync::mpsc::UnboundedReceiver<String> {
        self.prune_expired().await;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.sessions.lock().await.insert(
            id,
            SessionEntry {
                tx,
                last_touch: Instant::now(),
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
        let guard = self.sessions.lock().await;
        let Some(entry) = guard.get(id) else {
            return false;
        };
        entry.tx.send(data).is_ok()
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
        assert_eq!(rx.recv().await.unwrap(), "payload");
        reg.close("sess").await;
        assert!(!reg.is_open("sess").await);
        assert!(!reg.push("sess", "x".into()).await);
    }

    #[tokio::test]
    async fn expired_session_is_not_open() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_millis(1));
        let _rx = reg.open("sess".to_string()).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!reg.is_open("sess").await);
    }
}
