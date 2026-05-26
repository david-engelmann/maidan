//! HTTP CRUD handlers. Every handler returns `Result<Json<_>, ApiError>`
//! and lets the [`crate::error::ApiError`] type render the failure as
//! `application/problem+json`. Mutations publish an [`Event`] to the
//! bus after the store call succeeds.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use maidan_artifacts::Sha256;
use maidan_auth::{
    capability::{
        self, ARTIFACT_UPLOAD, MESSAGE_POST, SEARCH_QUERY, THREAD_TRANSITION, TOKEN_ADMIN,
        WORKSPACE_READ, WORKSPACE_WRITE,
    },
    hash_secret, AuthContext, TokenSecret,
};
use maidan_fsm::ThreadAction;
use maidan_store::Store;
use maidan_types::*;

use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::federation::PeerContext;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

async fn workspace_for_thread(
    store: &dyn Store,
    thread_id: ThreadId,
) -> Result<(WorkspaceId, ChannelId), ApiError> {
    let thread = store.get_thread(thread_id).await?;
    let channel = store.get_channel(thread.channel_id).await?;
    Ok((channel.workspace_id, channel.id))
}

async fn chain_for_message(
    store: &dyn Store,
    message_id: MessageId,
) -> Result<(WorkspaceId, ChannelId, ThreadId), ApiError> {
    let message = store.get_message(message_id).await?;
    let (workspace_id, channel_id) = workspace_for_thread(store, message.thread_id).await?;
    Ok((workspace_id, channel_id, message.thread_id))
}

// --- workspaces ---

pub async fn create_workspace(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateWorkspace>,
) -> ApiResult<(StatusCode, Json<Workspace>)> {
    if !state.auth_disabled && state.bootstrap_enabled {
        let count = state.store.count_workspaces().await?;
        if count > 0 {
            return Err(ApiError::Forbidden(
                "bootstrap only allows creating the first workspace; use bearer auth thereafter"
                    .into(),
            ));
        }
    }
    let ws = state
        .store
        .create_workspace(NewWorkspace { name: body.name })
        .await?;
    publish(
        &state,
        Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: ws.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(ws)))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Workspace>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.get_workspace(workspace_id).await?))
}

pub async fn list_events(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<ListEventsQuery>,
    auth: Option<Extension<AuthContext>>,
    peer: Option<Extension<PeerContext>>,
) -> ApiResult<Json<Vec<StoredEvent>>> {
    let workspace_id = WorkspaceId(workspace_id);
    match (&auth, &peer) {
        (Some(Extension(auth)), None) => {
            cap(auth, WORKSPACE_READ)?;
            ensure_workspace(auth, workspace_id)?;
        }
        (None, Some(Extension(PeerContext(peer)))) => {
            if peer.remote_workspace_id != workspace_id {
                return Err(ApiError::Forbidden(
                    "peer may only read its registered remote workspace".into(),
                ));
            }
        }
        _ => return Err(ApiError::Unauthorized),
    }
    Ok(Json(
        state
            .store
            .list_events_after(workspace_id, q.after_id, q.limit)
            .await?,
    ))
}

// --- members ---

pub async fn create_member(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMember>,
) -> ApiResult<(StatusCode, Json<Member>)> {
    let m = state
        .store
        .create_member(NewMember {
            workspace_id: WorkspaceId(workspace_id),
            handle: body.handle,
            display_name: body.display_name,
            kind: body.kind,
        })
        .await?;
    publish(
        &state,
        Event::MemberJoined {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(workspace_id),
            member: m.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_members(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Member>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_members(workspace_id).await?))
}

pub async fn get_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Member>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    Ok(Json(member))
}

pub async fn list_mentions_for_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListMentionsQuery>,
) -> ApiResult<Json<Vec<Mention>>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    Ok(Json(
        state
            .store
            .list_mentions_for_member(MemberId(id), q.limit)
            .await?,
    ))
}

// --- channels ---

pub async fn create_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateChannel>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let c = state
        .store
        .create_channel(NewChannel {
            workspace_id,
            name: body.name,
            topic: body.topic,
            private: body.private,
        })
        .await?;
    publish(
        &state,
        Event::ChannelCreated {
            occurred_at: Utc::now(),
            workspace_id,
            channel: c.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(c)))
}

pub async fn list_channels(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Channel>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_channels(workspace_id).await?))
}

pub async fn get_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Channel>> {
    let channel = state.store.get_channel(ChannelId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    Ok(Json(channel))
}

// --- threads ---

pub async fn create_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(channel_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateThread>,
) -> ApiResult<(StatusCode, Json<Thread>)> {
    let channel = state.store.get_channel(ChannelId(channel_id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    let t = state
        .store
        .create_thread(NewThread {
            channel_id: ChannelId(channel_id),
            parent_thread_id: body.parent_thread_id.map(ThreadId),
            title: body.title,
        })
        .await?;
    publish(
        &state,
        Event::ThreadCreated {
            occurred_at: Utc::now(),
            workspace_id: channel.workspace_id,
            channel_id: ChannelId(channel_id),
            thread: t.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(t)))
}

pub async fn list_threads(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(channel_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Thread>>> {
    let channel = state.store.get_channel(ChannelId(channel_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    Ok(Json(state.store.list_threads(ChannelId(channel_id)).await?))
}

pub async fn get_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Thread>> {
    let thread = state.store.get_thread(ThreadId(id)).await?;
    let channel = state.store.get_channel(thread.channel_id).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    Ok(Json(thread))
}

pub async fn transition_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<TransitionThread>,
) -> ApiResult<Json<Thread>> {
    cap(&auth, THREAD_TRANSITION)?;
    let action = ThreadAction::parse(&body.action).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "unknown action {:?}; expected start_review, close, or archive",
            body.action
        ))
    })?;
    let thread_id = ThreadId(id);
    let (workspace_id, channel_id) = workspace_for_thread(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, workspace_id)?;
    let result = state
        .store
        .transition_thread(thread_id, MemberId(body.actor_id), action)
        .await?;
    publish(
        &state,
        Event::ThreadStateChanged {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id,
            actor_id: MemberId(body.actor_id),
            from_state: result.from_state,
            to_state: result.to_state,
            thread: result.thread.clone(),
        },
    )
    .await;
    Ok(Json(result.thread))
}

// --- messages ---

pub async fn post_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    cap(&auth, MESSAGE_POST)?;
    let (workspace_id, channel_id) =
        workspace_for_thread(state.store.as_ref(), ThreadId(thread_id)).await?;
    ensure_workspace(&auth, workspace_id)?;
    let m = state
        .store
        .post_message(NewMessage {
            thread_id: ThreadId(thread_id),
            author_id: MemberId(body.author_id),
            body: body.body,
            metadata: body.metadata,
        })
        .await?;
    publish(
        &state,
        Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id: ThreadId(thread_id),
            message: m.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
    Query(q): Query<ListMessagesQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let (workspace_id, _) = workspace_for_thread(state.store.as_ref(), ThreadId(thread_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state
            .store
            .list_messages(ThreadId(thread_id), q.limit)
            .await?,
    ))
}

pub async fn get_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Message>> {
    let (workspace_id, _, _) = chain_for_message(state.store.as_ref(), MessageId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.get_message(MessageId(id)).await?))
}

pub async fn tombstone_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    let (workspace_id, channel_id, thread_id) =
        chain_for_message(state.store.as_ref(), MessageId(id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.tombstone_message(MessageId(id)).await?;
    publish(
        &state,
        Event::MessageTombstoned {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id,
            message_id: MessageId(id),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn purge_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    let (workspace_id, _, _) = chain_for_message(state.store.as_ref(), MessageId(id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.purge_message(MessageId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_mention(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMention>,
) -> ApiResult<StatusCode> {
    let (workspace_id, _channel_id, thread_id) =
        chain_for_message(state.store.as_ref(), MessageId(message_id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state
        .store
        .record_mention(MessageId(message_id), MemberId(body.member_id))
        .await?;
    publish(
        &state,
        Event::MentionRecorded {
            occurred_at: Utc::now(),
            workspace_id,
            thread_id,
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// --- votes ---

pub async fn cast_vote(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateVote>,
) -> ApiResult<StatusCode> {
    let (workspace_id, _channel_id, thread_id) =
        chain_for_message(state.store.as_ref(), MessageId(message_id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state
        .store
        .cast_vote(NewVote {
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
            kind: body.kind.clone(),
        })
        .await?;
    publish(
        &state,
        Event::VoteCast {
            occurred_at: Utc::now(),
            workspace_id,
            thread_id,
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
            vote_kind: body.kind,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_votes(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Vote>>> {
    let (workspace_id, _, _) =
        chain_for_message(state.store.as_ref(), MessageId(message_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state
            .store
            .list_votes_for_message(MessageId(message_id))
            .await?,
    ))
}

// --- artifacts ---

pub async fn upload_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<UploadArtifactQuery>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty artifact body".into()));
    }
    let sha = state.artifacts.put(body.clone()).await?;
    let artifact = state
        .store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_string(),
            size_bytes: body.len() as i64,
            mime_type: q.mime_type,
            kind: q.kind,
            uploaded_by: q.uploaded_by.map(MemberId),
        })
        .await?;
    publish(
        &state,
        Event::ArtifactUpserted {
            occurred_at: Utc::now(),
            artifact: artifact.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(artifact)))
}

pub async fn get_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(sha_hex): Path<String>,
) -> ApiResult<Response> {
    cap(&auth, WORKSPACE_READ)?;
    let sha = Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let meta = state.store.get_artifact_by_sha(&sha_hex).await?;
    let bytes = state.artifacts.get(&sha).await?;
    let mut headers = HeaderMap::new();
    if let Some(mime) = meta.mime_type {
        if let Ok(value) = mime.parse() {
            headers.insert(header::CONTENT_TYPE, value);
        }
    }
    headers.insert(
        header::HeaderName::from_static("x-artifact-kind"),
        meta.kind.as_str().parse().unwrap(),
    );
    Ok((headers, bytes).into_response())
}

pub async fn get_artifact_metadata(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(sha_hex): Path<String>,
) -> ApiResult<Json<Artifact>> {
    cap(&auth, WORKSPACE_READ)?;
    Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(state.store.get_artifact_by_sha(&sha_hex).await?))
}

// --- references ---

pub async fn create_reference(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<CreateReference>,
) -> ApiResult<(StatusCode, Json<Reference>)> {
    cap(&auth, WORKSPACE_WRITE)?;
    let r = state
        .store
        .add_reference(NewReference {
            src_kind: body.src_kind,
            src_id: body.src_id,
            dst_kind: body.dst_kind,
            dst_id: body.dst_id,
            relation: body.relation,
        })
        .await?;
    publish(
        &state,
        Event::ReferenceAdded {
            occurred_at: Utc::now(),
            reference: r.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(r)))
}

pub async fn list_references(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListReferencesQuery>,
) -> ApiResult<Json<Vec<Reference>>> {
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(
        state
            .store
            .list_references_from(q.src_kind, q.src_id)
            .await?,
    ))
}

// --- search ---

pub async fn search_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<maidan_search::SearchHit>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SEARCH_QUERY)?;
    ensure_workspace(&auth, workspace_id)?;
    let filters = maidan_search::SearchFilters {
        author_id: q.author.map(MemberId),
        channel_id: q.channel.map(ChannelId),
        author_kind: q.kind,
    };
    let hits = match q.mode {
        SearchMode::Lexical => {
            state
                .search
                .search_messages(workspace_id, &q.q, q.limit, &filters)
                .await?
        }
        SearchMode::Semantic => {
            if q.author.is_some() || q.channel.is_some() || q.kind.is_some() {
                return Err(ApiError::BadRequest(
                    "facets are only supported for lexical search (mode=lexical)".into(),
                ));
            }
            let embedding = state
                .embedding_provider
                .embed(&q.q)
                .map_err(|e| ApiError::Internal(format!("embedding generation failed: {e}")))?;
            state
                .search
                .semantic_search(workspace_id, &embedding, q.limit)
                .await?
        }
    };
    Ok(Json(hits))
}

// --- api tokens ---

pub async fn mint_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, member_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<MintApiToken>,
) -> ApiResult<(StatusCode, Json<MintApiTokenResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    let member_id = MemberId(member_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    let member = state.store.get_member(member_id).await?;
    if member.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "member does not belong to workspace".into(),
        ));
    }

    let capabilities = if body.capabilities.is_empty() {
        capability::default_minted()
    } else {
        capability::validate_list(&body.capabilities).map_err(ApiError::BadRequest)?;
        body.capabilities
    };

    let secret = TokenSecret::generate();
    let record = state
        .store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            token_hash: hash_secret(secret.as_str()),
            label: body.label,
            capabilities,
            expires_at: body.expires_at,
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(MintApiTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            member_id: record.member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
        }),
    ))
}

pub async fn revoke_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ApiToken>> {
    cap(&auth, TOKEN_ADMIN)?;
    let token_id = ApiTokenId(id);
    let existing = state.store.get_api_token(token_id).await?;
    ensure_workspace(&auth, existing.workspace_id)?;
    Ok(Json(state.store.revoke_api_token(token_id).await?))
}

/// Fire-and-forget event publish. Errors are logged but never surfaced
/// to the HTTP caller — the store has already committed, and the bus
/// being temporarily unavailable should not turn a successful mutation
/// into a 5xx.
pub(crate) async fn publish(state: &AppState, event: Event) {
    let stored = match state.store.append_event(&event).await {
        Ok(row) => row,
        Err(err) => {
            tracing::warn!(error = %err, "event log append failed");
            return;
        }
    };
    let envelope = BusEnvelope {
        log_id: stored.id,
        event,
    };
    if let Err(err) = state.bus.publish(envelope).await {
        tracing::warn!(error = %err, "bus publish failed");
    }
}
