//! SSE event stream for MCP-style reactive clients (`GET /mcp/stream`).

use std::convert::Infallible;
use std::sync::{atomic::AtomicI64, Arc};
use std::time::Duration;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    Extension,
};
use maidan_auth::{capability::EVENT_SUBSCRIBE, AuthContext};
use maidan_types::EventFilter;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::error::ApiError;
use crate::event_stream::{
    self, emit_replay_truncated_if_needed, replay_matching_events, subscribe_ack_payload,
};
use crate::state::AppState;
use crate::subscribe_resume;

#[derive(Debug, Deserialize)]
pub struct McpStreamQuery {
    #[serde(default)]
    pub workspace_id: Option<uuid::Uuid>,
    /// Replay persisted events with `id > after_id` before live bus delivery.
    #[serde(default)]
    pub after_id: i64,
    #[serde(default)]
    pub resume_token: Option<String>,
    #[serde(default)]
    pub consumer_id: Option<String>,
    #[serde(default)]
    pub dm_conversation_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub channel_grants: Vec<uuid::Uuid>,
    /// Narrow live delivery to a single channel / thread / member (Cluster 150),
    /// so an MCP agent can subscribe to just one thread or "just my mentions"
    /// server-side instead of filtering the whole workspace client-side.
    #[serde(default)]
    pub channel_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub thread_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub member_id: Option<uuid::Uuid>,
    /// Comma-separated event kinds (snake_case), e.g. `message_posted,mention_recorded`.
    #[serde(default)]
    pub kinds: Option<String>,
    /// Opt into gap-free at-least-once delivery (Cluster 126): cursor-driven
    /// reconcile instead of the optimistic live path. Requires `workspace_id`
    /// and `consumer_id`; adds a stability-window latency floor on fresh events.
    #[serde(default)]
    pub at_least_once: bool,
    /// Opt into lean event frames (Cluster 178, token round 3): `{log_id, kind,
    /// ...ids}` pointers instead of full serialized events.
    #[serde(default)]
    pub lean: bool,
}

pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<McpStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    auth.require_capability(EVENT_SUBSCRIBE)
        .map_err(|_| ApiError::Forbidden("missing event:subscribe capability".into()))?;

    let (mut filter, mut after_id, from_resume_token) = resolve_stream_params(&state, &q, &auth)?;
    crate::dm::expand_event_filter(&state, &mut filter).await?;
    crate::subscribe_grants::apply_subscribe_grants(&state, &auth, &mut filter)
        .await
        .map_err(ApiError::BadRequest)?;
    if let Some(ref consumer_id) = q.consumer_id {
        crate::delivery::validate_consumer_id(consumer_id).map_err(ApiError::BadRequest)?;
        after_id = crate::delivery::effective_subscribe_after_id(
            state.store.as_ref(),
            Some(consumer_id.as_str()),
            filter.workspace_id,
            after_id,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let delivery_consumer_id = q.consumer_id.clone();
    // At-least-once requires both a workspace filter and a durable consumer id
    // (the reconcile cursor is keyed by them); ignore the flag otherwise.
    let reconcile = (q.at_least_once)
        .then(|| filter.workspace_id.zip(delivery_consumer_id.clone()))
        .flatten();

    let (sse_tx, sse_rx) = mpsc::channel(256);
    let (text_tx, mut text_rx) = mpsc::channel::<String>(256);

    tokio::spawn(async move {
        while let Some(payload) = text_rx.recv().await {
            if sse_tx
                .send(Ok(Event::default().data(payload)))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let lean = q.lean;
    let mut high_water = after_id;
    // At-least-once mode delivers the backlog via the reconcile loop's first
    // pass (stability-gated), so skip the optimistic replay here.
    if reconcile.is_none() && (after_id > 0 || from_resume_token) {
        let outcome = replay_matching_events(
            state.store.as_ref(),
            &filter,
            after_id,
            &text_tx,
            delivery_consumer_id.as_deref(),
            lean,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        high_water = outcome.high_water;
        emit_replay_truncated_if_needed(
            &text_tx,
            outcome.high_water,
            filter.workspace_id,
            outcome.truncated,
        )
        .await;
    }

    let token = subscribe_resume::sign_resume_token(
        &filter,
        high_water,
        state.subscribe_resume_secret(),
        state.subscribe_resume_ttl_secs,
    )
    .map_err(|e| ApiError::Internal(format!("resume token: {e}")))?;
    let ack = subscribe_ack_payload(&token, high_water)
        .ok_or_else(|| ApiError::Internal("subscribe_ack serialization failed".into()))?;
    if text_tx.send(ack).await.is_err() {
        return Err(ApiError::Internal(
            "stream closed before subscribe_ack".into(),
        ));
    }

    let bus_filter = filter.clone();
    let subscriber = state
        .bus
        .subscribe(filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let bus_store = state.store.clone();
    if let Some((workspace_id, consumer_id)) = reconcile {
        let stability = state.delivery_stability;
        let interval = state.delivery_reconcile_interval;
        tokio::spawn(async move {
            event_stream::reconcile_deliver(
                subscriber,
                text_tx,
                bus_store,
                bus_filter,
                workspace_id,
                consumer_id,
                high_water,
                stability,
                interval,
                lean,
            )
            .await;
        });
    } else {
        let watermark = Arc::new(AtomicI64::new(high_water));
        tokio::spawn(async move {
            event_stream::forward_bus_items(
                subscriber,
                text_tx,
                watermark,
                bus_store,
                bus_filter,
                crate::subscribe_metrics::SubscribeTransport::McpSse,
                delivery_consumer_id,
                lean,
            )
            .await;
        });
    }

    let stream = ReceiverStream::new(sse_rx);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

fn resolve_stream_params(
    state: &AppState,
    q: &McpStreamQuery,
    auth: &AuthContext,
) -> Result<(EventFilter, i64, bool), ApiError> {
    if let Some(token) = q.resume_token.as_deref().filter(|t| !t.is_empty()) {
        if state.subscribe_resume_secret.is_none() && state.oidc.is_none() {
            return Err(ApiError::Internal(
                "subscribe resume not configured on server".into(),
            ));
        }
        let (filter, after_id) =
            subscribe_resume::verify_resume_token(token, state.subscribe_resume_secret())
                .map_err(|e| ApiError::BadRequest(format!("invalid resume_token: {e}")))?;
        if after_id > 0 && filter.workspace_id.is_none() {
            return Err(ApiError::BadRequest(
                "resume token requires workspace_id for replay".into(),
            ));
        }
        if let Some(ws) = filter.workspace_id {
            auth.ensure_workspace(ws)
                .map_err(|_| ApiError::Forbidden("token is not valid for this workspace".into()))?;
        }
        return Ok((filter, after_id, true));
    }

    if q.after_id < 0 {
        return Err(ApiError::BadRequest("after_id must be non-negative".into()));
    }

    let mut filter = EventFilter::all();
    if let Some(ws) = q.workspace_id {
        filter.workspace_id = Some(maidan_types::WorkspaceId(ws));
        auth.ensure_workspace(maidan_types::WorkspaceId(ws))
            .map_err(|_| ApiError::Forbidden("token is not valid for this workspace".into()))?;
    } else if q.after_id > 0 {
        return Err(ApiError::BadRequest(
            "after_id requires workspace_id query parameter".into(),
        ));
    }
    if let Some(dm) = q.dm_conversation_id {
        filter.dm_conversation_id = Some(maidan_types::DmConversationId(dm));
    }
    if !q.channel_grants.is_empty() {
        filter.channel_grants = Some(
            q.channel_grants
                .iter()
                .copied()
                .map(maidan_types::ChannelId)
                .collect(),
        );
    }
    if let Some(ch) = q.channel_id {
        filter.channel_id = Some(maidan_types::ChannelId(ch));
    }
    if let Some(th) = q.thread_id {
        filter.thread_id = Some(maidan_types::ThreadId(th));
    }
    if let Some(m) = q.member_id {
        filter.member_id = Some(maidan_types::MemberId(m));
    }
    if let Some(kinds) = q.kinds.as_deref().filter(|s| !s.is_empty()) {
        let mut set = std::collections::HashSet::new();
        for name in kinds.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let kind = maidan_types::EventKind::parse(name)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown event kind: {name}")))?;
            set.insert(kind);
        }
        if !set.is_empty() {
            filter.kinds = Some(set);
        }
    }

    Ok((filter, q.after_id, false))
}
