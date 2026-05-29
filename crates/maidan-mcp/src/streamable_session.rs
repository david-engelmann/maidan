//! Active `POST /mcp/streamable` sessions (`Mcp-Session-Id`, Cluster 35).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

#[derive(Clone)]
pub struct StreamableSessionRegistry {
    senders: Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>,
}

impl Default for StreamableSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamableSessionRegistry {
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Open a new session and return the SSE consumer side. Returns `None` if `id` is already open.
    pub async fn open(&self, id: String) -> tokio::sync::mpsc::UnboundedReceiver<String> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.senders.lock().await.insert(id, tx);
        rx
    }

    pub async fn is_open(&self, id: &str) -> bool {
        self.senders.lock().await.contains_key(id)
    }

    pub async fn push(&self, id: &str, data: String) -> bool {
        let guard = self.senders.lock().await;
        let Some(tx) = guard.get(id) else {
            return false;
        };
        tx.send(data).is_ok()
    }

    pub async fn close(&self, id: &str) {
        self.senders.lock().await.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_push_and_close() {
        let reg = StreamableSessionRegistry::new();
        let mut rx = reg.open("sess".to_string()).await;
        assert!(reg.is_open("sess").await);
        assert!(reg.push("sess", "payload".into()).await);
        assert_eq!(rx.recv().await.unwrap(), "payload");
        reg.close("sess").await;
        assert!(!reg.is_open("sess").await);
        assert!(!reg.push("sess", "x".into()).await);
    }
}
