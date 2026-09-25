//! Dedicated, read-only consumer surface for cross-organization share tickets.
//! Share secrets never become [`maidan_auth::AuthContext`] values and these
//! routes are deliberately outside the ordinary bearer-authenticated router.

use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use maidan_artifacts::Sha256;
use maidan_auth::hash_secret;
use maidan_types::{
    ArtifactKind, Channel, ContentBlock, MemberId, MessageId, ShareTicket, ShareTicketId, ThreadId,
    ThreadState, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::ApiError;
use crate::state::AppState;

const SHARE_SECRET_PREFIX: &str = "maid_share_";
const DEFAULT_PAGE_LIMIT: i64 = 50;
const MAX_PAGE_LIMIT: i64 = 100;

#[derive(Clone)]
pub struct ShareTicketContext(pub ShareTicket);

fn parse_share_authorization(value: &str) -> Option<&str> {
    let (scheme, secret) = value.split_once(' ')?;
    let secret = secret.trim();
    if !scheme.eq_ignore_ascii_case("ShareTicket")
        || !secret.starts_with(SHARE_SECRET_PREFIX)
        || secret.len() != SHARE_SECRET_PREFIX.len() + 64
        || !secret[SHARE_SECRET_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(secret)
}

fn harden_share_response(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(header::VARY, HeaderValue::from_static("Authorization"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

/// Resolve `Authorization: ShareTicket maid_share_…` without invoking the API
/// token/peer/session resolvers. Invalid, expired, and revoked tickets share one
/// response so the endpoint is not a ticket-state oracle.
pub async fn middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let secret = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_share_authorization);
    let Some(secret) = secret else {
        return harden_share_response(ApiError::Unauthorized.into_response());
    };
    let ticket = match state
        .store
        .resolve_share_ticket(&hash_secret(secret), Utc::now())
        .await
    {
        Ok(ticket) => ticket,
        Err(error) => {
            if !matches!(error, maidan_store::StoreError::NotFound) {
                tracing::error!(error = %error, "share_ticket.resolve_failed");
            }
            return harden_share_response(ApiError::Unauthorized.into_response());
        }
    };
    let workspace_id = ticket.workspace_id;
    request.extensions_mut().insert(ShareTicketContext(ticket));
    let mut response = harden_share_response(next.run(request).await);
    response
        .extensions_mut()
        .insert(crate::room_lsn::RoomScope(workspace_id));
    response
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SharedArtifact {
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub kind: ArtifactKind,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShareManifest {
    pub ticket_id: ShareTicketId,
    pub workspace_id: WorkspaceId,
    pub owner_id: MemberId,
    pub expires_at: DateTime<Utc>,
    pub channel: Channel,
    pub artifacts: Vec<SharedArtifact>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SharedThread {
    pub id: ThreadId,
    pub parent_thread_id: Option<ThreadId>,
    pub title: Option<String>,
    pub state: ThreadState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SharedThreadPage {
    pub items: Vec<SharedThread>,
    pub next_cursor: Option<ThreadId>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SharedMessage {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub author_id: MemberId,
    pub body: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ContentBlock>>,
    pub posted_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SharedMessagePage {
    pub items: Vec<SharedMessage>,
    pub next_cursor: Option<MessageId>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SharePageQuery {
    pub cursor: Option<uuid::Uuid>,
    pub limit: Option<i64>,
}

fn page_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

pub async fn manifest(
    State(state): State<AppState>,
    Extension(context): Extension<ShareTicketContext>,
) -> Result<Json<ShareManifest>, ApiError> {
    let ticket = context.0;
    let channel = state.store.get_channel(ticket.channel_id).await?;
    if channel.tombstoned_at.is_some() {
        return Err(ApiError::NotFound);
    }
    let shas = state.store.list_share_ticket_artifacts(ticket.id).await?;
    let mut artifacts = Vec::with_capacity(shas.len());
    for sha256 in shas {
        let artifact = state
            .store
            .get_artifact_for_workspace(ticket.workspace_id, &sha256)
            .await?;
        if artifact.tombstoned_at.is_none() {
            artifacts.push(SharedArtifact {
                sha256: artifact.sha256,
                size_bytes: artifact.size_bytes,
                mime_type: artifact.mime_type,
                kind: artifact.kind,
            });
        }
    }
    Ok(Json(ShareManifest {
        ticket_id: ticket.id,
        workspace_id: ticket.workspace_id,
        owner_id: ticket.owner_id,
        expires_at: ticket.expires_at,
        channel,
        artifacts,
    }))
}

pub async fn list_threads(
    State(state): State<AppState>,
    Extension(context): Extension<ShareTicketContext>,
    Query(query): Query<SharePageQuery>,
) -> Result<Json<SharedThreadPage>, ApiError> {
    let limit = page_limit(query.limit);
    let mut rows = state
        .store
        .page_threads_for_channel(context.0.channel_id, query.cursor.map(ThreadId), limit + 1)
        .await?;
    let next_cursor = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|thread| thread.id)
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|thread| SharedThread {
            id: thread.id,
            parent_thread_id: thread.parent_thread_id,
            title: thread.title,
            state: thread.state,
            created_at: thread.created_at,
            updated_at: thread.updated_at,
        })
        .collect();
    Ok(Json(SharedThreadPage { items, next_cursor }))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Extension(context): Extension<ShareTicketContext>,
    Path(thread_id): Path<uuid::Uuid>,
    Query(query): Query<SharePageQuery>,
) -> Result<Json<SharedMessagePage>, ApiError> {
    let thread_id = ThreadId(thread_id);
    let thread = state.store.get_thread(thread_id).await?;
    if thread.channel_id != context.0.channel_id || thread.tombstoned_at.is_some() {
        return Err(ApiError::NotFound);
    }
    let limit = page_limit(query.limit);
    let mut rows = state
        .store
        .list_messages_after(thread_id, query.cursor.map(MessageId), limit + 1)
        .await?;
    let next_cursor = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|message| message.id)
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|message| SharedMessage {
            id: message.id,
            thread_id: message.thread_id,
            author_id: message.author_id,
            body: message.body,
            metadata: message.metadata,
            content: message.content,
            posted_at: message.posted_at,
            edited_at: message.edited_at,
        })
        .collect();
    Ok(Json(SharedMessagePage { items, next_cursor }))
}

pub async fn download_artifact(
    State(state): State<AppState>,
    Extension(context): Extension<ShareTicketContext>,
    Path(sha_hex): Path<String>,
) -> Result<Response, ApiError> {
    let sha = Sha256::from_hex(&sha_hex).map_err(|_| ApiError::NotFound)?;
    if !state
        .store
        .share_ticket_allows_artifact(context.0.id, &sha_hex, Utc::now())
        .await?
    {
        return Err(ApiError::NotFound);
    }
    let artifact = state
        .store
        .get_artifact_for_workspace(context.0.workspace_id, &sha_hex)
        .await?;
    if artifact.tombstoned_at.is_some() {
        return Err(ApiError::NotFound);
    }
    let bytes = state.artifacts.get(&sha).await?;
    let mut headers = HeaderMap::new();
    if let Some(mime_type) = artifact.mime_type {
        if let Ok(value) = mime_type.parse() {
            headers.insert(header::CONTENT_TYPE, value);
        }
    }
    if let Ok(kind) = artifact.kind.as_str().parse() {
        headers.insert(header::HeaderName::from_static("x-artifact-kind"), kind);
    }
    Ok((headers, bytes).into_response())
}

#[cfg(test)]
mod tests {
    use super::parse_share_authorization;

    #[test]
    fn share_authorization_has_a_distinct_strict_scheme() {
        let secret = format!("maid_share_{}", "a".repeat(64));
        assert_eq!(
            parse_share_authorization(&format!("ShareTicket {secret}")),
            Some(secret.as_str())
        );
        assert_eq!(
            parse_share_authorization(&format!("shareticket {secret}")),
            Some(secret.as_str())
        );
        assert!(parse_share_authorization(&format!("Bearer {secret}")).is_none());
        assert!(parse_share_authorization("ShareTicket maid_share_short").is_none());
    }
}
