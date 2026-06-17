//! HTTP CRUD handlers. Every handler returns `Result<Json<_>, ApiError>`
//! and lets the [`crate::error::ApiError`] type render the failure as
//! `application/problem+json`. Mutations publish an [`Event`] to the
//! bus after the store call succeeds.
//!
//! Handlers are grouped by domain into submodules; this module re-exports
//! each submodule's items so every existing `crate::routes::<name>` path
//! keeps resolving unchanged.

use maidan_auth::{
    capability::{MESSAGE_POST, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_router::route_mentions_for_message;
use maidan_types::*;

use crate::error::ApiError;
use crate::state::AppState;
use chrono::Utc;

mod artifact;
mod channel;
mod member;
mod message;
mod reference;
mod search;
mod social;
mod thread;
mod token;
mod workspace;

pub use artifact::*;
pub use channel::*;
pub use member::*;
pub use message::*;
pub use reference::*;
pub use search::*;
pub use social::*;
pub use thread::*;
pub use token::*;
pub use workspace::*;

pub(crate) type ApiResult<T> = Result<T, ApiError>;

pub(crate) fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

pub(crate) fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

pub(crate) fn ensure_message_edit(
    auth: &AuthContext,
    editor_id: MemberId,
    author_id: MemberId,
) -> ApiResult<()> {
    if auth.bypass {
        return Ok(());
    }
    if editor_id == author_id {
        cap(auth, MESSAGE_POST)
    } else {
        cap(auth, WORKSPACE_WRITE)
    }
}

pub(crate) async fn publish_routed_mentions(
    state: &AppState,
    thread_id: ThreadId,
    workspace_id: WorkspaceId,
    message: &Message,
) {
    let mentioned = match route_mentions_for_message(
        state.store.as_ref(),
        message.id,
        message.author_id,
        &message.body,
    )
    .await
    {
        Ok(ids) => ids,
        Err(err) => {
            tracing::warn!(error = %err, "mention routing failed");
            return;
        }
    };
    for member_id in mentioned {
        publish(
            state,
            Event::MentionRecorded {
                occurred_at: Utc::now(),
                workspace_id,
                thread_id,
                message_id: message.id,
                member_id,
            },
        )
        .await;
    }
}

/// Fire-and-forget event publish. Errors are logged but never surfaced
/// to the HTTP caller — the store has already committed, and the bus
/// being temporarily unavailable should not turn a successful mutation
/// into a 5xx.
///
/// Returns the new `log_id` when append succeeded.
pub(crate) async fn publish(state: &AppState, event: Event) -> Option<i64> {
    let stored = match state.store.append_event(&event).await {
        Ok(row) => row,
        Err(err) => {
            tracing::warn!(error = %err, "event log append failed");
            return None;
        }
    };
    if state.outbox_relay {
        // Wake an idle relay promptly (Cluster 108). Capacity-1 channel: a
        // `Full` error means a nudge is already queued, which is enough.
        if let Some(nudge) = &state.outbox_nudge {
            let _ = nudge.try_send(());
        }
        return Some(stored.id);
    }
    let envelope = BusEnvelope {
        log_id: stored.id,
        event,
    };
    if let Err(err) = state.bus.publish(envelope).await {
        tracing::warn!(error = %err, "bus publish failed");
    }
    Some(stored.id)
}

#[cfg(test)]
mod publish_tests {
    use std::sync::Arc;

    use chrono::Utc;
    use maidan_artifacts::LocalFsStore;
    use maidan_bus::{test_support::RecordingBus, InMemoryBus};
    use maidan_search::SqliteSearch;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use maidan_types::*;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::publish;
    use crate::state::AppState;

    async fn sqlite_state(bus: Arc<dyn maidan_bus::EventBus>) -> AppState {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("fk");
        run_sqlite_migrations(&pool).await.expect("migrate");
        let store = Arc::new(SqliteStore::new(pool.clone()));
        let search: Arc<dyn maidan_search::Search> = Arc::new(SqliteSearch::new(pool));
        let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
        AppState::for_tests(store, artifacts, bus, search)
    }

    #[tokio::test]
    async fn publish_calls_bus_when_outbox_relay_disabled() {
        let inner = Arc::new(InMemoryBus::new());
        let bus = Arc::new(RecordingBus::new(inner));
        let mut state = sqlite_state(bus.clone()).await;
        state.outbox_relay = false;

        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "pub-ws".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        };
        let log_id = publish(&state, event).await;
        assert!(log_id.is_some());
        assert_eq!(bus.publishes(), 1);
    }

    #[tokio::test]
    async fn publish_skips_bus_when_outbox_relay_enabled() {
        let inner = Arc::new(InMemoryBus::new());
        let bus = Arc::new(RecordingBus::new(inner));
        let mut state = sqlite_state(bus.clone()).await;
        state.outbox_relay = true;

        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "defer-ws".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        };
        let log_id = publish(&state, event).await;
        assert!(log_id.is_some());
        assert_eq!(bus.publishes(), 0);
    }
}
