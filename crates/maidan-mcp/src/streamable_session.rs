//! Active `POST /mcp/streamable` sessions, keyed by `Mcp-Session-Id` and
//! expired on a TTL. A session belongs to the caller that opened it: another
//! caller presenting its id is treated as presenting no session at all.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::subscriptions::Principal;

/// Per-session SSE buffer bound. A slow client that fills this is disconnected
/// (push returns false) instead of growing server memory without limit — the
/// channel was previously unbounded.
const SESSION_BUFFER: usize = 256;

/// How many recently-sent events to retain per session for `Last-Event-ID`
/// replay on reconnect. Bounded like the live buffer.
const SESSION_LOG_CAP: usize = 256;

/// The most replay bytes retained for one session. A single larger frame is
/// still offered live, and is not kept in the log.
const MAX_SESSION_LOG_BYTES: usize = 4 * 1024 * 1024;

/// The most sessions held at once. Opening one more evicts the least recently
/// touched, so an abandoned-session pile-up costs bounded memory and never
/// turns a live client away.
const MAX_SESSIONS: usize = 10_000;

struct SessionEntry {
    owner: Principal,
    tx: tokio::sync::mpsc::Sender<(u64, String)>,
    last_touch: Instant,
    /// Monotonic SSE event id assigned to the next pushed message.
    next_event_id: u64,
    /// Recent `(event_id, payload)` frames, for `Last-Event-ID` replay.
    log: VecDeque<(u64, String)>,
    /// Sum of `log` payload lengths. Kept beside the deque so eviction does
    /// not walk every frame.
    log_bytes: usize,
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

    /// Drop expired sessions and return how many remain. Expiry used to run
    /// only as a side effect of open / lookup / push, so a server that stopped
    /// receiving MCP traffic kept every dead session forever; the server's
    /// metrics tick now calls this, and reports the count.
    pub async fn reap(&self) -> usize {
        let mut sessions = self.sessions.lock().await;
        prune_locked(&mut sessions, self.ttl);
        sessions.len()
    }

    /// Open a new session and return the SSE consumer side. Each item is the
    /// `(event_id, payload)` pair; the transport renders the id as the SSE
    /// `id:` field so a client can resume with `Last-Event-ID`.
    pub async fn open(
        &self,
        id: String,
        owner: Principal,
    ) -> tokio::sync::mpsc::Receiver<(u64, String)> {
        let (tx, rx) = tokio::sync::mpsc::channel(SESSION_BUFFER);
        let mut sessions = self.sessions.lock().await;
        prune_locked(&mut sessions, self.ttl);
        if sessions.len() >= MAX_SESSIONS {
            let stalest = sessions
                .iter()
                .min_by_key(|(_, entry)| entry.last_touch)
                .map(|(id, _)| id.clone());
            if let Some(stalest) = stalest {
                sessions.remove(&stalest);
            }
        }
        sessions.insert(
            id,
            SessionEntry {
                owner,
                tx,
                last_touch: Instant::now(),
                next_event_id: 0,
                log: VecDeque::new(),
                log_bytes: 0,
            },
        );
        rx
    }

    pub async fn touch(&self, id: &str) {
        let mut sessions = self.sessions.lock().await;
        prune_locked(&mut sessions, self.ttl);
        if let Some(entry) = sessions.get_mut(id) {
            entry.last_touch = Instant::now();
        }
    }

    /// Whether `id` is an open session that `owner` opened.
    pub async fn is_open_for(&self, id: &str, owner: &Principal) -> bool {
        let mut sessions = self.sessions.lock().await;
        prune_locked(&mut sessions, self.ttl);
        sessions.get(id).is_some_and(|entry| &entry.owner == owner)
    }

    /// The ids of every unexpired session.
    pub async fn open_ids(&self) -> HashSet<String> {
        let mut sessions = self.sessions.lock().await;
        prune_locked(&mut sessions, self.ttl);
        sessions.keys().cloned().collect()
    }

    pub async fn push(&self, id: &str, data: String) -> bool {
        let mut guard = self.sessions.lock().await;
        prune_locked(&mut guard, self.ttl);
        let Some(entry) = guard.get_mut(id) else {
            return false;
        };
        // Assign the event id and record the frame for replay before delivering,
        // so a client that reconnects with `Last-Event-ID` can recover it even
        // if the live consumer had already dropped.
        let event_id = entry.next_event_id;
        entry.next_event_id += 1;
        if data.len() > MAX_SESSION_LOG_BYTES {
            // One oversized frame would pin more than the budget by itself.
            // Live delivery still happens below; replay does not keep it.
            entry.log.clear();
            entry.log_bytes = 0;
        } else {
            while entry.log.len() >= SESSION_LOG_CAP
                || entry.log_bytes + data.len() > MAX_SESSION_LOG_BYTES
            {
                match entry.log.pop_front() {
                    Some((_, old)) => {
                        entry.log_bytes = entry.log_bytes.saturating_sub(old.len());
                    }
                    None => {
                        entry.log_bytes = 0;
                        break;
                    }
                }
            }
            entry.log_bytes += data.len();
            entry.log.push_back((event_id, data.clone()));
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
    /// order — the `Last-Event-ID` replay set. Empty for an unknown session or
    /// when nothing newer was retained.
    pub async fn replay_after(&self, id: &str, after: u64) -> Vec<(u64, String)> {
        let mut guard = self.sessions.lock().await;
        prune_locked(&mut guard, self.ttl);
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

    /// Close `id` if `owner` opened it; whether it did.
    pub async fn close_for(&self, id: &str, owner: &Principal) -> bool {
        let mut sessions = self.sessions.lock().await;
        prune_locked(&mut sessions, self.ttl);
        if sessions.get(id).is_some_and(|entry| &entry.owner == owner) {
            sessions.remove(id);
            true
        } else {
            false
        }
    }
}

fn prune_locked(sessions: &mut HashMap<String, SessionEntry>, ttl: Duration) {
    sessions.retain(|_, entry| entry.last_touch.elapsed() < ttl);
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

    const OWNER: Principal = Principal::Bypass;

    #[tokio::test]
    async fn open_push_and_close() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let mut rx = reg.open("sess".to_string(), OWNER).await;
        assert!(reg.is_open_for("sess", &OWNER).await);
        assert!(reg.push("sess", "payload".into()).await);
        assert_eq!(rx.recv().await.unwrap(), (0, "payload".to_string()));
        assert!(reg.close_for("sess", &OWNER).await);
        assert!(!reg.is_open_for("sess", &OWNER).await);
        assert!(!reg.push("sess", "x".into()).await);
    }

    #[tokio::test]
    async fn a_session_is_open_only_to_the_caller_that_opened_it() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let _rx = reg.open("sess".to_string(), OWNER).await;
        let stranger = Principal::Caller {
            workspace_id: maidan_types::WorkspaceId(uuid::Uuid::new_v4()),
            member_id: maidan_types::MemberId(uuid::Uuid::new_v4()),
            actor_id: maidan_types::MemberId(uuid::Uuid::new_v4()),
            token_id: None,
            app_installation_id: None,
            delegation_grant_id: None,
        };
        assert!(!reg.is_open_for("sess", &stranger).await);
        assert!(!reg.close_for("sess", &stranger).await);
        assert!(reg.is_open_for("sess", &OWNER).await);
    }

    #[tokio::test]
    async fn replay_after_returns_only_newer_frames() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let _rx = reg.open("sess".to_string(), OWNER).await;
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
        let _rx = reg.open("sess".to_string(), OWNER).await; // never drained
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
        let _rx = reg.open("sess".to_string(), OWNER).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!reg.is_open_for("sess", &OWNER).await);
    }

    async fn expired() -> StreamableSessionRegistry {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_millis(1));
        let _rx = reg.open("sess".to_string(), OWNER).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        reg
    }

    #[tokio::test]
    async fn touch_replay_and_close_do_not_resurrect_an_expired_session() {
        let reg = expired().await;
        reg.touch("sess").await;
        assert!(reg.replay_after("sess", 0).await.is_empty());
        assert!(!reg.close_for("sess", &OWNER).await);
        assert!(!reg.is_open_for("sess", &OWNER).await);
        assert!(reg.open_ids().await.is_empty());
        assert_eq!(reg.reap().await, 0);
    }

    #[tokio::test]
    async fn the_replay_log_drops_frames_past_the_byte_budget() {
        let reg = StreamableSessionRegistry::with_ttl(Duration::from_secs(60));
        let mut rx = reg.open("sess".to_string(), OWNER).await;
        let chunk = "x".repeat(3 * 1024 * 1024);
        assert!(reg.push("sess", "first".into()).await);
        assert!(reg.push("sess", chunk.clone()).await);
        assert!(reg.push("sess", chunk.clone()).await);
        // ids 0, 1, 2. Two 3 MiB frames cannot share a 4 MiB log, so id 1 is
        // evicted when id 2 is recorded. id 0 went with it.
        let replay = reg.replay_after("sess", 0).await;
        assert_eq!(
            replay.len(),
            1,
            "{:?}",
            replay.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );
        assert_eq!(replay[0].0, 2);
        assert_eq!(replay[0].1.len(), chunk.len());

        let huge = "y".repeat(MAX_SESSION_LOG_BYTES + 1);
        assert!(reg.push("sess", huge.clone()).await);
        assert!(
            reg.replay_after("sess", 0).await.is_empty(),
            "a frame larger than the budget is not replayed"
        );
        let mut live = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            live.push(frame);
        }
        assert!(
            live.iter().any(|(id, body)| *id == 3 && body == &huge),
            "the oversized frame is still delivered live"
        );
    }
}
