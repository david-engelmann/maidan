//! Federation ingress, peer admin, and shared ingest logic.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use maidan_a2a::{FederatedEventBatch, FederationEnvelope, FederationError};
use maidan_auth::{
    capability::{FEDERATION_ADMIN, FEDERATION_INGEST},
    decrypt_peer_secret, encrypt_peer_secret, hash_secret, resolve_peer_bearer, AuthContext,
    TokenSecret,
};
use maidan_types::{Event, NewPeer, Peer, PeerId, WorkspaceId};
use serde::Serialize;
use utoipa::ToSchema;

use crate::dto::{CreatePeer, MintPeerResponse, PeerResponse};
use crate::error::{ApiError, ApiJson};
use crate::routes::publish;
use crate::state::AppState;

/// Authenticated federation peer (ingress or event-tail read).
#[derive(Debug, Clone)]
pub struct PeerContext(pub Peer);

type ApiResult<T> = Result<T, ApiError>;

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

pub fn federation_disabled_from_env() -> bool {
    matches!(
        std::env::var("FEDERATION_DISABLED").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

pub fn poll_interval_secs_from_env() -> u64 {
    std::env::var("FEDERATION_POLL_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
}

pub fn remember_peer_secret(
    secrets: &Arc<RwLock<HashMap<PeerId, String>>>,
    peer_id: PeerId,
    secret: String,
) {
    if let Ok(mut guard) = secrets.write() {
        guard.insert(peer_id, secret);
    }
}

pub fn forget_peer_secret(secrets: &Arc<RwLock<HashMap<PeerId, String>>>, peer_id: PeerId) {
    if let Ok(mut guard) = secrets.write() {
        guard.remove(&peer_id);
    }
}

pub async fn hydrate_federation_secrets(state: &AppState) -> Result<(), String> {
    let Some(key) = state.federation.encryption_key.as_deref() else {
        return Ok(());
    };
    let peers = state
        .store
        .list_enabled_peers()
        .await
        .map_err(|e| e.to_string())?;
    for peer in peers {
        let Some(ciphertext) = peer.outbound_secret_ciphertext.as_deref() else {
            continue;
        };
        match decrypt_peer_secret(ciphertext, key) {
            Ok(secret) => remember_peer_secret(&state.federation.outbound_secrets, peer.id, secret),
            Err(err) => tracing::warn!(
                peer = %peer.id,
                error = %err,
                "failed to decrypt stored federation peer secret"
            ),
        }
    }
    Ok(())
}

pub fn resolve_outbound_secret(state: &AppState, peer: &Peer) -> Option<String> {
    if let Ok(guard) = state.federation.outbound_secrets.read() {
        if let Some(secret) = guard.get(&peer.id) {
            return Some(secret.clone());
        }
    }
    let ciphertext = peer.outbound_secret_ciphertext.as_deref()?;
    let key = state.federation.encryption_key.as_deref()?;
    let secret = decrypt_peer_secret(ciphertext, key).ok()?;
    remember_peer_secret(&state.federation.outbound_secrets, peer.id, secret.clone());
    Some(secret)
}

pub async fn ingest_events(
    State(state): State<AppState>,
    Extension(PeerContext(peer)): Extension<PeerContext>,
    ApiJson(batch): ApiJson<FederatedEventBatch>,
) -> ApiResult<Json<IngestSummary>> {
    batch.validate().map_err(federation_err)?;
    let mut ingested = 0u32;
    let mut skipped = 0u32;
    for envelope in batch.events {
        match ingest_envelope(&state, &peer, envelope).await? {
            IngestOutcome::Ingested => ingested += 1,
            IngestOutcome::SkippedDuplicate => skipped += 1,
        }
    }
    Ok(Json(IngestSummary { ingested, skipped }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IngestSummary {
    pub ingested: u32,
    pub skipped: u32,
}

pub(crate) enum IngestOutcome {
    Ingested,
    SkippedDuplicate,
}

pub(crate) async fn ingest_envelope(
    state: &AppState,
    peer: &Peer,
    envelope: FederationEnvelope,
) -> ApiResult<IngestOutcome> {
    envelope.validate().map_err(federation_err)?;
    if envelope.origin_peer_id != peer.id {
        return Err(ApiError::Forbidden(
            "origin_peer_id does not match authenticated peer".into(),
        ));
    }
    if state
        .store
        .federated_ingest_exists(peer.id, envelope.remote_event_id)
        .await?
    {
        return Ok(IngestOutcome::SkippedDuplicate);
    }

    let mut event = event_from_stored(&envelope.event)?;
    event = remap_event_workspace(event, peer.workspace_id);
    let Some(log_id) = publish(state, event).await else {
        return Err(ApiError::Internal("event log append failed".into()));
    };
    let recorded = state
        .store
        .try_record_federated_ingest(peer.id, envelope.remote_event_id, log_id)
        .await?;
    if !recorded {
        return Ok(IngestOutcome::SkippedDuplicate);
    }
    let consumer_id = crate::delivery::federation_consumer_id(peer.id);
    if let Err(err) = state
        .store
        .advance_delivery_cursor(&consumer_id, peer.workspace_id, log_id)
        .await
    {
        tracing::warn!(
            error = %err,
            peer = %peer.id,
            log_id,
            "federation delivery cursor advance failed"
        );
    }
    Ok(IngestOutcome::Ingested)
}

fn event_from_stored(stored: &maidan_types::StoredEvent) -> ApiResult<Event> {
    serde_json::from_value(stored.payload.clone())
        .map_err(|e| ApiError::BadRequest(format!("invalid event payload: {e}")))
}

fn remap_event_workspace(event: Event, workspace_id: WorkspaceId) -> Event {
    use Event::*;
    match event {
        WorkspaceCreated {
            occurred_at,
            mut workspace,
        } => {
            workspace.id = workspace_id;
            WorkspaceCreated {
                occurred_at,
                workspace,
            }
        }
        MemberJoined {
            occurred_at,
            member,
            ..
        } => MemberJoined {
            occurred_at,
            workspace_id,
            member,
        },
        ChannelCreated {
            occurred_at,
            mut channel,
            ..
        } => {
            channel.workspace_id = workspace_id;
            ChannelCreated {
                occurred_at,
                workspace_id,
                channel,
            }
        }
        ThreadCreated {
            occurred_at,
            workspace_id: _,
            channel_id,
            thread,
        } => ThreadCreated {
            occurred_at,
            workspace_id,
            channel_id,
            thread,
        },
        ThreadStateChanged {
            occurred_at,
            workspace_id: _,
            channel_id,
            thread_id,
            actor_id,
            from_state,
            to_state,
            thread,
        } => ThreadStateChanged {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            actor_id,
            from_state,
            to_state,
            thread,
        },
        MessagePosted {
            occurred_at,
            workspace_id: _,
            channel_id,
            thread_id,
            dm_conversation_id,
            message,
        } => MessagePosted {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            dm_conversation_id,
            message,
        },
        MessageEdited {
            occurred_at,
            workspace_id: _,
            channel_id,
            thread_id,
            dm_conversation_id,
            editor_id,
            message,
        } => MessageEdited {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            dm_conversation_id,
            editor_id,
            message,
        },
        MessageTombstoned {
            occurred_at,
            workspace_id: _,
            channel_id,
            thread_id,
            dm_conversation_id,
            message_id,
        } => MessageTombstoned {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            dm_conversation_id,
            message_id,
        },
        MentionRecorded {
            occurred_at,
            workspace_id: _,
            thread_id,
            message_id,
            member_id,
        } => MentionRecorded {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
        },
        VoteCast {
            occurred_at,
            workspace_id: _,
            thread_id,
            message_id,
            member_id,
            vote_kind,
        } => VoteCast {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
            vote_kind,
        },
        ReactionAdded {
            occurred_at,
            workspace_id: _,
            thread_id,
            message_id,
            member_id,
            emoji,
        } => ReactionAdded {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
            emoji,
        },
        ReactionRemoved {
            occurred_at,
            workspace_id: _,
            thread_id,
            message_id,
            member_id,
            emoji,
        } => ReactionRemoved {
            occurred_at,
            workspace_id,
            thread_id,
            message_id,
            member_id,
            emoji,
        },
        MessagePinned {
            occurred_at,
            workspace_id: _,
            channel_id,
            thread_id,
            message_id,
            member_id,
        } => MessagePinned {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            message_id,
            member_id,
        },
        MessageUnpinned {
            occurred_at,
            workspace_id: _,
            channel_id,
            thread_id,
            message_id,
            member_id,
        } => MessageUnpinned {
            occurred_at,
            workspace_id,
            channel_id,
            thread_id,
            message_id,
            member_id,
        },
        ReferenceAdded {
            occurred_at,
            reference,
        } => ReferenceAdded {
            occurred_at,
            reference,
        },
        ArtifactUpserted {
            occurred_at,
            artifact,
        } => ArtifactUpserted {
            occurred_at,
            artifact,
        },
    }
}

fn federation_err(err: FederationError) -> ApiError {
    match err {
        FederationError::Unauthorized => ApiError::Unauthorized,
        other => ApiError::BadRequest(other.to_string()),
    }
}

pub async fn create_peer(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreatePeer>,
) -> ApiResult<(StatusCode, Json<MintPeerResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, FEDERATION_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    maidan_a2a::validate_peer_name(&body.name).map_err(federation_err)?;
    maidan_a2a::validate_base_url(&body.base_url).map_err(federation_err)?;

    let secret = TokenSecret::generate();
    let key = state.federation.encryption_key.as_deref().ok_or_else(|| {
        ApiError::Internal(
            "FEDERATION_ENCRYPTION_KEY must be set to create federation peers".into(),
        )
    })?;
    let outbound_secret_ciphertext =
        encrypt_peer_secret(secret.as_str(), key).map_err(|e| ApiError::Internal(e.to_string()))?;
    let remote_workspace_id = body
        .remote_workspace_id
        .map(WorkspaceId)
        .unwrap_or(workspace_id);
    let peer = state
        .store
        .create_peer(NewPeer {
            workspace_id,
            remote_workspace_id,
            name: body.name,
            base_url: body.base_url,
            token_hash: hash_secret(secret.as_str()),
            outbound_secret_ciphertext: Some(outbound_secret_ciphertext),
        })
        .await?;
    remember_peer_secret(
        &state.federation.outbound_secrets,
        peer.id,
        secret.as_str().to_string(),
    );

    Ok((
        StatusCode::CREATED,
        Json(MintPeerResponse {
            peer: PeerResponse::from(peer),
            secret: secret.as_str().to_string(),
        }),
    ))
}

pub async fn list_peers(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<PeerResponse>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, FEDERATION_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let peers = state.store.list_peers(workspace_id).await?;
    Ok(Json(peers.into_iter().map(PeerResponse::from).collect()))
}

pub async fn delete_peer(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, peer_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(workspace_id);
    let peer_id = PeerId(peer_id);
    cap(&auth, FEDERATION_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let peer = state.store.get_peer(peer_id).await?;
    if peer.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    state.store.delete_peer(peer_id).await?;
    forget_peer_secret(&state.federation.outbound_secrets, peer_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WellKnownMcp {
    pub http: String,
    pub streamable: String,
    pub notifications: String,
    pub stream: String,
    pub protocol_version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WellKnownMaidan {
    pub name: String,
    pub version: String,
    pub mcp: WellKnownMcp,
    pub a2a: WellKnownA2a,
    pub agent_card: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WellKnownA2a {
    pub ingress: String,
    pub protocol_rpc: String,
    pub protocol_version: String,
}

pub async fn well_known() -> impl IntoResponse {
    Json(WellKnownMaidan {
        name: "maidan".to_string(),
        version: crate::version().to_string(),
        mcp: WellKnownMcp {
            http: "/mcp".to_string(),
            streamable: "/mcp/streamable".to_string(),
            notifications: "/mcp/notifications".to_string(),
            stream: "/mcp/stream".to_string(),
            protocol_version: "2024-11-05".to_string(),
        },
        a2a: WellKnownA2a {
            ingress: "/a2a/v1/events".to_string(),
            protocol_rpc: "/a2a/v1/rpc".to_string(),
            protocol_version: "1.0".to_string(),
        },
        agent_card: "/.well-known/agent-card.json".to_string(),
        capabilities: vec![FEDERATION_INGEST.to_string(), FEDERATION_ADMIN.to_string()],
    })
}

pub async fn peer_auth_middleware(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let Some(secret) = crate::auth::bearer_from_headers(req.headers()) else {
        return ApiError::Unauthorized.into_response();
    };
    match resolve_peer_bearer(state.store.as_ref(), secret).await {
        Ok(peer) => {
            req.extensions_mut().insert(PeerContext(peer));
            next.run(req).await
        }
        Err(_) => ApiError::Unauthorized.into_response(),
    }
}
