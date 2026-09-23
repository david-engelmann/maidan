//! HTTP CRUD handlers. Every handler returns `Result<Json<_>, ApiError>`
//! and lets the [`crate::error::ApiError`] type render the failure as
//! `application/problem+json`. Mutations publish an [`Event`] to the
//! bus after the store call succeeds.
//!
//! Handlers are grouped by domain into submodules; this module re-exports
//! each submodule's items so every existing `crate::routes::<name>` path
//! keeps resolving unchanged.

use maidan_auth::{capability::MESSAGE_POST, AuthContext};
use maidan_router::{parse_at_handles, route_mentions_in_message};
use maidan_types::*;

use crate::error::ApiError;
use crate::state::AppState;
use chrono::Utc;

mod approval_gate;
mod artifact;
mod channel;
mod egress_ops;
mod egress_targets;
mod freeze;
mod glossary;
mod land_gate;
mod mail_ops;
mod member;
mod memory_block;
mod message;
mod recipe;
mod reference;
mod review;
mod room;
mod search;
mod secret;
mod share_ticket;
mod skills;
mod social;
mod task_schedule;
mod thread;
mod token;
mod workspace;

pub use approval_gate::*;
pub use artifact::*;
pub use channel::*;
pub use egress_ops::*;
pub use egress_targets::*;
pub use freeze::*;
pub use glossary::*;
pub use land_gate::*;
pub use mail_ops::*;
pub use member::*;
pub use memory_block::*;
pub use message::*;
pub use recipe::*;
pub use reference::*;
pub use review::*;
pub use room::*;
pub use search::*;
pub use secret::*;
pub use share_ticket::*;
pub use skills::*;
pub use social::*;
pub use task_schedule::*;
pub use thread::*;
pub use token::*;
pub use workspace::*;

pub(crate) type ApiResult<T> = Result<T, ApiError>;

pub(crate) fn clamp_context_transition_limit(limit: i64) -> i64 {
    if limit > 0 {
        limit.min(200)
    } else {
        50
    }
}

pub(crate) fn cap(auth: &AuthContext, capability: &'static str) -> ApiResult<()> {
    maidan_auth::require_observed_capability(
        auth,
        maidan_auth::AuthorizationSurface::Rest,
        capability,
    )
    .map_err(Into::into)
}

pub(crate) fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

/// Personal state — inbox, notification preferences, delivery address, follows,
/// push subscriptions — belongs to one member, and a token acts as one member.
/// Rewriting another member's delivery address is not that; it is how their
/// digest mail gets redirected.
///
/// There is no override. Acting for someone else means holding a delegated
/// token for them, whose `member_id` *is* that member — so it passes this check
/// as itself, and every use is durably recorded with the delegate as actor.
pub(crate) fn ensure_own_personal_state(auth: &AuthContext, claimed: MemberId) -> ApiResult<()> {
    if auth.bypass || claimed == auth.member_id {
        return Ok(());
    }
    Err(ApiError::Forbidden("member_id is not yours".into()))
}

/// Whether `member` is at (or over) their workspace's WIP limit: they already
/// hold ≥ the cap in concurrent live claims, so `claim`/ `claim_next` must not
/// hand them more. `false` when the workspace has no limit set (unlimited). A
/// soft limit — the count and the limit are read on the primary but not in the
/// claim's transaction, so a member self-racing two concurrent claims could
/// momentarily reach limit+1; the occupancy view shows the truth.
pub(crate) async fn at_wip_limit(
    store: &dyn maidan_store::Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> ApiResult<bool> {
    match store.get_wip_limit(workspace_id).await? {
        Some(limit) => Ok(store.count_live_claims(member_id).await? >= limit),
        None => Ok(false),
    }
}

pub(crate) fn ensure_message_edit(
    auth: &AuthContext,
    editor_id: MemberId,
    author_id: MemberId,
) -> ApiResult<()> {
    if auth.bypass {
        return Ok(());
    }
    // A message is shown under its author's name, and an edit keeps it there.
    // Anyone else's edit would put words in the author's mouth — the record
    // would say they wrote what they did not. Moderating someone else's
    // message means removing it (tombstone, purge), never rewriting it.
    if editor_id != author_id {
        return Err(ApiError::Forbidden(
            "only a message's author can edit it; another member's message can be tombstoned, not rewritten"
                .into(),
        ));
    }
    cap(auth, MESSAGE_POST)
}

pub(crate) async fn publish_routed_mentions(
    state: &AppState,
    thread_id: ThreadId,
    workspace_id: WorkspaceId,
    message: &Message,
) {
    // The common case is a post with no `@handles` — short-circuit before any
    // store work. Otherwise route with the workspace the caller already
    // resolved, so we skip the per-post `resolve_message_chain` round-trip that
    // `route_mentions_for_message` did only to re-derive that same workspace
    // id.
    if parse_at_handles(&message.body).is_empty() {
        return;
    }
    let mentioned = match route_mentions_in_message(
        state.store.as_ref(),
        workspace_id,
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
/// conceding a lost event. Transient store errors (pool timeout, brief lock
/// contention) are the common failure and usually clear on retry.
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
/// failure is benign — the event is already logged (and in relay mode the
/// outbox will deliver it), so it should never turn a successful mutation into
/// a 5xx. A durable **append** failure is not benign: the domain row committed
/// but the event would be lost (no notifications, no delivery, no indexing). So
/// the append is retried on transient errors; a hard failure after retries is
/// logged loudly and counted (`maidan_event_append_failures_total`) for
/// alerting. True single-transaction atomicity of the domain write and the
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
        // Wake an idle relay promptly. Capacity-1 channel: a `Full` error means
        // a nudge is already queued, which is enough.
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

/// Pass a mutation's result through, first recording a `ThreadSpawnDenied`
/// event when the store refused the spawn.
///
/// The refusal already reaches the caller as a 409 — this is the *operator's*
/// view: which member keeps pushing a claim past its fan-out cap, on the same
/// event stream as everything else in the room. The store owns the gate but has
/// no author on the thread-create path, so the route supplies `actor`.
/// Best-effort by construction: [`publish`] already swallows a bus hiccup, and
/// a lost append cannot make the refusal any less refused.
pub(crate) async fn observe_spawn_denial<T>(
    state: &AppState,
    actor: Option<MemberId>,
    result: Result<T, maidan_store::StoreError>,
) -> ApiResult<T> {
    let err = match result {
        Ok(value) => return Ok(value),
        Err(err) => err,
    };
    if let maidan_store::StoreError::SpawnRejected(denial) = &err {
        publish(state, denial.denied_event(actor)).await;
    }
    Err(err.into())
}

/// Notify the bus for an event that was **already appended durably** inside the
/// mutation's transaction. Unlike [`publish`], there is no durable append here
/// — the domain row and the event were committed atomically by the
/// `*_with_event` store method — so this is purely the best-effort live
/// notification, and a bus/relay hiccup can never undo a committed mutation.
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

    use super::{clamp_context_transition_limit, publish, retry};
    use crate::state::AppState;

    #[test]
    fn context_transition_limit_matches_the_public_contract() {
        assert_eq!(clamp_context_transition_limit(i64::MIN), 50);
        assert_eq!(clamp_context_transition_limit(0), 50);
        assert_eq!(clamp_context_transition_limit(50), 50);
        assert_eq!(clamp_context_transition_limit(i64::MAX), 200);
    }

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
}
