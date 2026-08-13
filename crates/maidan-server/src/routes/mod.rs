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

/// Anti-spoofing guard (Cluster 202): a **session** caller (browser/OIDC login,
/// no API token) may only act as its *own* member. `claimed` is the
/// caller-supplied acting member — the `author_id`/`actor_id`/`editor_id`/voter
/// on a member-attributed write. A **bearer token** is the orchestrator model
/// and may legitimately act as any member in its workspace (unchanged);
/// `bypass` (auth disabled / tests) is unrestricted. This centralizes the guard
/// that previously lived only on `post_message`.
pub(crate) fn ensure_acting_member(auth: &AuthContext, claimed: MemberId) -> ApiResult<()> {
    if !auth.bypass && auth.token_id.is_none() && claimed != auth.member_id {
        return Err(ApiError::Forbidden(
            "a session caller may only act as its own member".into(),
        ));
    }
    Ok(())
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

/// Extra attempts to re-append a domain event after the first fails, before
/// conceding a lost event (Cluster 184). Transient store errors (pool timeout,
/// brief lock contention) are the common failure and usually clear on retry.
const EVENT_APPEND_ATTEMPTS: u32 = 3;
const EVENT_APPEND_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

/// Retry an async fallible op up to `attempts` times (min 1), sleeping `backoff`
/// between tries. Returns the first `Ok`, else the final `Err`.
async fn retry<T, E, F, Fut>(mut op: F, attempts: u32, backoff: std::time::Duration) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let max = attempts.max(1);
    let mut done = 0u32;
    loop {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                done += 1;
                if done >= max {
                    return Err(e);
                }
                if !backoff.is_zero() {
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
}

/// Publish a domain event after the mutation's store call succeeded.
///
/// The **durable append** and the **bus notify** are treated differently. A bus
/// failure is benign — the event is already logged (and in relay mode the outbox
/// will deliver it), so it should never turn a successful mutation into a 5xx.
/// A durable **append** failure is not benign: the domain row committed but the
/// event would be lost (no notifications, no delivery, no indexing). So the
/// append is retried on transient errors (Cluster 184); a hard failure after
/// retries is logged loudly and counted (`maidan_event_append_failures_total`)
/// for alerting. True single-transaction atomicity of the domain write and the
/// event append is a larger refactor tracked in Open Work.
///
/// Returns the new `log_id` when the append succeeded.
pub(crate) async fn publish(state: &AppState, event: Event) -> Option<i64> {
    // First attempt on the hot path borrows `event` (no clone). Only on failure
    // do we clone for the (rare) retry loop.
    let stored = match state.store.append_event(&event).await {
        Ok(row) => row,
        Err(first) => {
            tracing::warn!(error = %first, "event log append failed; retrying");
            let store = state.store.clone();
            let ev = event.clone();
            let retried = retry(
                move || {
                    let store = store.clone();
                    let ev = ev.clone();
                    async move { store.append_event(&ev).await }
                },
                EVENT_APPEND_ATTEMPTS,
                EVENT_APPEND_BACKOFF,
            )
            .await;
            match retried {
                Ok(row) => row,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        "event.append_failed: domain row committed but event lost after retries"
                    );
                    crate::metrics::record_event_append_failure();
                    return None;
                }
            }
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

/// Notify the bus for an event that was **already appended durably** inside the
/// mutation's transaction (Cluster 205 transactional outbox). Unlike [`publish`],
/// there is no durable append here — the domain row and the event were committed
/// atomically by the `*_with_event` store method — so this is purely the
/// best-effort live notification, and a bus/relay hiccup can never undo a
/// committed mutation.
pub(crate) async fn publish_stored(state: &AppState, stored: StoredEvent) {
    if state.outbox_relay {
        if let Some(nudge) = &state.outbox_nudge {
            let _ = nudge.try_send(());
        }
        return;
    }
    // In-memory / notify bus: hydrate the event from the stored payload.
    match serde_json::from_value::<Event>(stored.payload) {
        Ok(event) => {
            let envelope = BusEnvelope {
                log_id: stored.id,
                event,
            };
            if let Err(err) = state.bus.publish(envelope).await {
                tracing::warn!(error = %err, "bus publish failed");
            }
        }
        Err(err) => {
            tracing::error!(error = %err, "publish_stored: event payload deserialize failed");
        }
    }
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

    use super::{publish, retry};
    use crate::state::AppState;

    #[tokio::test]
    async fn retry_returns_ok_after_transient_failures() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        let out: Result<u32, &str> = retry(
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n < 2 {
                        Err("transient")
                    } else {
                        Ok(n)
                    }
                }
            },
            5,
            std::time::Duration::ZERO,
        )
        .await;
        assert_eq!(out, Ok(2));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_gives_up_and_returns_last_error() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        let out: Result<u32, &str> = retry(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move { Err::<u32, &str>("always") }
            },
            3,
            std::time::Duration::ZERO,
        )
        .await;
        assert_eq!(out, Err("always"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

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

    #[test]
    fn ensure_acting_member_blocks_session_spoof_only() {
        use super::ensure_acting_member;
        use maidan_auth::AuthContext;

        let me = MemberId(uuid::Uuid::new_v4());
        let other = MemberId(uuid::Uuid::new_v4());
        let ws = WorkspaceId(uuid::Uuid::new_v4());

        // Session caller (browser/OIDC, no token): may act only as itself.
        let session = AuthContext::from_session(me, ws, vec![]);
        assert!(ensure_acting_member(&session, me).is_ok());
        assert!(
            ensure_acting_member(&session, other).is_err(),
            "a session caller cannot act as another member"
        );

        // Bearer token: the orchestrator model — may act as any member (unchanged).
        let bearer = AuthContext::from_token(ApiTokenId(uuid::Uuid::new_v4()), me, ws, vec![]);
        assert!(ensure_acting_member(&bearer, other).is_ok());

        // Bypass (auth disabled / tests): unrestricted.
        assert!(ensure_acting_member(&AuthContext::bypass(), other).is_ok());
    }
}
