//! `/health` endpoint. Reports the status of every external dependency
//! the server needs to be useful — currently the DB and the artifact
//! store. Returns 200 only when both subsystems respond.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use maidan_artifacts::Sha256;
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;
use crate::version;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub db: SubsystemStatus,
    pub storage: SubsystemStatus,
    pub indexer: SubsystemStatus,
    pub indexer_last_event_at: Option<DateTime<Utc>>,
    pub bus: SubsystemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingStatus>,
    pub version: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EmbeddingStatus {
    pub model: String,
    pub dimension: usize,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemStatus {
    Ok,
    Error(String),
}

impl SubsystemStatus {
    fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

pub async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

pub async fn ready(state: State<AppState>) -> impl IntoResponse {
    readiness(state).await
}

pub async fn handler(state: State<AppState>) -> impl IntoResponse {
    readiness(state).await
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let body = collect(&state).await;
    let code = if body.status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(body))
}

/// Every readiness check, as `/health/ready` reports it; `/operator/status`
/// shows the same result so the two cannot disagree.
pub(crate) async fn collect(state: &AppState) -> HealthResponse {
    // Shutting down: not ready, whatever the dependencies say, so the pod leaves
    // the Service endpoints while it still serves what it already accepted.
    let draining = state.draining.load(Ordering::Relaxed);
    let db = match state.store.health_check().await {
        Ok(()) => SubsystemStatus::Ok,
        Err(e) => SubsystemStatus::Error(e.to_string()),
    };

    let storage = check_artifact_store(state).await;
    let (indexer, indexer_last_event_at) = check_indexer(state).await;
    let bus = check_bus(state);
    let embedding = Some(EmbeddingStatus {
        model: state.embedding_provider.model_name().to_string(),
        dimension: state.embedding_provider.dimension(),
    });

    let healthy = !draining && db.is_ok() && storage.is_ok() && indexer.is_ok() && bus.is_ok();
    HealthResponse {
        status: if draining {
            "draining".to_string()
        } else if healthy {
            "ok".to_string()
        } else {
            "degraded".to_string()
        },
        db,
        storage,
        indexer,
        indexer_last_event_at,
        bus,
        embedding,
        version: version(),
    }
}

fn check_bus(state: &AppState) -> SubsystemStatus {
    match &state.bus_listener_health {
        None => SubsystemStatus::Ok,
        Some(health) => match health.check() {
            Ok(()) => SubsystemStatus::Ok,
            Err(msg) => SubsystemStatus::Error(msg),
        },
    }
}

async fn check_indexer(state: &AppState) -> (SubsystemStatus, Option<DateTime<Utc>>) {
    let stale_secs = std::env::var("INDEXER_STALE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    check_indexer_at(state, stale_secs, Utc::now().timestamp_millis()).await
}

/// `check_indexer` with the threshold and the clock passed in.
///
/// Silence alone is not a fault: an instance with nothing to index has no
/// indexer events either, and would be unready for as long as it was quiet.
/// The indexer is degraded only when the log holds an indexable event it has
/// not handled **and** neither that event nor the indexer's last progress is
/// newer than the threshold. Progress counts because a long backlog that is
/// being worked through has an old oldest-pending event but is not stuck.
async fn check_indexer_at(
    state: &AppState,
    stale_secs: i64,
    now_ms: i64,
) -> (SubsystemStatus, Option<DateTime<Utc>>) {
    if let Some(err) = state.indexer_last_error.read().await.clone() {
        return (
            SubsystemStatus::Error(format!("embedding indexer error: {err}")),
            None,
        );
    }
    let ms = state.indexer_last_event_unix_ms.load(Ordering::Relaxed);
    if ms == 0 {
        return (SubsystemStatus::Ok, None);
    }
    let at = DateTime::from_timestamp_millis(ms);
    if stale_secs > 0 {
        let pending = match indexer_backlog(state).await {
            Ok(pending) => pending,
            // The database check reports the outage; do not also blame the
            // indexer for a query that could not run.
            Err(err) => {
                tracing::debug!(error = %err, "indexer backlog probe failed");
                None
            }
        };
        if let Some(pending_at) = pending {
            let idle_secs = (now_ms - ms) / 1000;
            let pending_secs = (now_ms - pending_at.timestamp_millis()) / 1000;
            if idle_secs > stale_secs && pending_secs > stale_secs {
                return (
                    SubsystemStatus::Error(format!(
                        "indexer is behind: oldest unindexed event is {pending_secs}s old and \
                         the indexer made no progress for {idle_secs}s (threshold {stale_secs}s)"
                    )),
                    at,
                );
            }
        }
    }
    (SubsystemStatus::Ok, at)
}

/// When the oldest indexable event the indexer has not handled was stored, or
/// `None` when it is caught up. One `MAX(id)` read answers the common case; the
/// kind-filtered probe runs only while the log is ahead of the indexer.
pub(crate) async fn indexer_backlog(
    state: &AppState,
) -> Result<Option<DateTime<Utc>>, maidan_store::StoreError> {
    let processed = state.indexer_processed_log_id.load(Ordering::Relaxed);
    if state.store.max_event_id().await? <= processed {
        return Ok(None);
    }
    Ok(state
        .store
        .oldest_event_after_of_kinds(processed, maidan_types::SEARCH_PROJECTOR_KINDS)
        .await?
        .map(|(_, at)| at))
}

async fn check_artifact_store(state: &AppState) -> SubsystemStatus {
    // Use a deterministic probe sha that we never write; existence check
    // is sufficient to verify the backend is reachable and not panicking.
    let probe = Sha256::compute(b"maidan-health-probe");
    match state.artifacts.exists(&probe).await {
        Ok(_) => SubsystemStatus::Ok,
        Err(e) => SubsystemStatus::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_search::SqliteSearch;
    use maidan_store::{prelude::*, run_sqlite_migrations, SqliteStore};
    use maidan_types::{MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace};
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    const THRESHOLD: i64 = 300;
    const DAY_MS: i64 = 86_400_000;

    struct Harness {
        state: AppState,
        store: Arc<SqliteStore>,
        ws: maidan_types::WorkspaceId,
        thread: maidan_types::ThreadId,
        author: maidan_types::MemberId,
        _dir: tempfile::TempDir,
    }

    async fn harness() -> Harness {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("pragma");
        run_sqlite_migrations(&pool).await.expect("migrate");
        let store = Arc::new(SqliteStore::for_tests(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "w".into() })
            .await
            .expect("ws");
        let author = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "a".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .expect("member");
        let ch = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "c".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        let (thread, _) = store
            .create_thread_with_event(NewThread {
                channel_id: ch.id,
                parent_thread_id: None,
                title: None,
            })
            .await
            .expect("thread");
        let dir = tempfile::tempdir().expect("tempdir");
        let state = AppState::for_tests(
            store.clone(),
            Arc::new(LocalFsStore::new(dir.path())),
            Arc::new(maidan_bus::InMemoryBus::new()),
            Arc::new(SqliteSearch::new(pool)),
        );
        Harness {
            state,
            store,
            ws: ws.id,
            thread: thread.id,
            author: author.id,
            _dir: dir,
        }
    }

    impl Harness {
        async fn post(&self) -> i64 {
            let (_, ev) = self
                .store
                .post_message_with_event(
                    NewMessage {
                        thread_id: self.thread,
                        author_id: self.author,
                        body: "hello".into(),
                        metadata: serde_json::json!({}),
                        content: None,
                    },
                    None,
                )
                .await
                .expect("post");
            ev.id
        }

        /// The indexer last progressed at `ms` and has handled up to `log_id`.
        fn indexer_at(&self, ms: i64, log_id: i64) {
            self.state
                .indexer_last_event_unix_ms
                .store(ms, Ordering::Relaxed);
            self.state
                .indexer_processed_log_id
                .store(log_id, Ordering::Relaxed);
        }

        /// The store's own stamp for `log_id`, so no host-vs-database clock
        /// comparison decides a result.
        async fn stored_at_ms(&self, log_id: i64) -> i64 {
            let (_, at) = self
                .store
                .oldest_event_after_of_kinds(log_id - 1, maidan_types::SEARCH_PROJECTOR_KINDS)
                .await
                .expect("probe")
                .expect("event");
            at.timestamp_millis()
        }
    }

    #[tokio::test]
    async fn an_idle_indexer_past_the_threshold_is_ready() {
        let h = harness().await;
        let id = h.post().await;
        let posted = h.stored_at_ms(id).await;
        h.indexer_at(posted, id);
        let (status, _) = check_indexer_at(&h.state, THRESHOLD, posted + DAY_MS).await;
        assert!(
            status.is_ok(),
            "nothing to index is not a fault: {status:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_log_is_ready_whatever_the_threshold() {
        let h = harness().await;
        h.indexer_at(1, 0);
        let (status, _) = check_indexer_at(&h.state, THRESHOLD, DAY_MS * 400).await;
        assert!(status.is_ok(), "{status:?}");
    }

    #[tokio::test]
    async fn events_the_indexer_never_handled_degrade_it_past_the_threshold() {
        let h = harness().await;
        let id = h.post().await;
        let posted = h.stored_at_ms(id).await;
        h.indexer_at(posted - 1000, id - 1);
        let (status, _) =
            check_indexer_at(&h.state, THRESHOLD, posted + (THRESHOLD + 10) * 1000).await;
        match status {
            SubsystemStatus::Error(msg) => assert!(msg.contains("behind"), "{msg}"),
            other => panic!("expected degraded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pending_events_within_the_threshold_are_ready() {
        let h = harness().await;
        let id = h.post().await;
        let posted = h.stored_at_ms(id).await;
        h.indexer_at(posted - DAY_MS, id - 1);
        let (status, _) = check_indexer_at(&h.state, THRESHOLD, posted + 5_000).await;
        assert!(
            status.is_ok(),
            "an old heartbeat with fresh work is an idle indexer about to wake: {status:?}"
        );
    }

    #[tokio::test]
    async fn a_backlog_the_indexer_is_working_through_is_ready() {
        let h = harness().await;
        let id = h.post().await;
        let posted = h.stored_at_ms(id).await;
        let now = posted + DAY_MS;
        h.indexer_at(now - 1_000, id - 1);
        let (status, _) = check_indexer_at(&h.state, THRESHOLD, now).await;
        assert!(status.is_ok(), "recent progress is not a stall: {status:?}");
    }

    #[tokio::test]
    async fn events_of_other_kinds_are_not_a_backlog() {
        let h = harness().await;
        let id = h.post().await;
        let posted = h.stored_at_ms(id).await;
        h.indexer_at(posted, id);
        h.store
            .create_channel(NewChannel {
                workspace_id: h.ws,
                name: "later".into(),
                topic: None,
                private: false,
            })
            .await
            .expect("channel");
        let (status, _) = check_indexer_at(&h.state, THRESHOLD, posted + DAY_MS).await;
        assert!(status.is_ok(), "{status:?}");
    }

    #[tokio::test]
    async fn an_indexer_error_still_degrades_an_idle_instance() {
        let h = harness().await;
        h.indexer_at(1, 0);
        *h.state.indexer_last_error.write().await = Some("provider timeout".into());
        let (status, _) = check_indexer_at(&h.state, 0, DAY_MS).await;
        match status {
            SubsystemStatus::Error(msg) => assert!(msg.contains("embedding indexer error")),
            other => panic!("expected degraded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_zero_threshold_never_degrades_on_lag() {
        let h = harness().await;
        let id = h.post().await;
        let posted = h.stored_at_ms(id).await;
        h.indexer_at(posted - DAY_MS, 0);
        let (status, _) = check_indexer_at(&h.state, 0, posted + DAY_MS).await;
        assert!(status.is_ok(), "{status:?}");
    }
}
