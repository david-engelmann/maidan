//! Operator DLQ for the durable mail outbox (Cluster 306): list dead-lettered
//! notification emails and requeue one for another delivery attempt.
//!
//! **Per-workspace since Cluster 398.3.** These rows carry `to_address`,
//! `subject` and the message body, and the query used to be global behind
//! `token:admin` — which is minted per workspace — so any workspace admin could
//! read every other tenant's outbound email. The caller now sees its own
//! workspace; `operator:global` widens that to the whole instance, and is the
//! only way to reach a row whose workspace is `NULL`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{OPERATOR_GLOBAL, TOKEN_ADMIN},
    AuthContext,
};
use maidan_types::{DeadMail, MailOutboxId};
use serde::Deserialize;

use super::{cap, ApiResult};
use crate::error::ApiError;
use crate::state::AppState;

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Deserialize)]
pub struct ListDeadMailQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

/// The DLQ scope for this caller (Cluster 398.3).
///
/// `token:admin` is minted per workspace, so the default view is the caller's
/// own workspace. `operator:global` widens it to every row — including the ones
/// with a `NULL` workspace, which predate this cluster or have no tenant
/// context and so cannot be attributed to anybody.
fn dlq_scope(auth: &AuthContext) -> Option<maidan_types::WorkspaceId> {
    (!auth.bypass && !auth.has_capability(OPERATOR_GLOBAL)).then_some(auth.workspace_id)
}

/// `GET /operator/mail/dead` — dead-lettered outbox entries for the caller's
/// workspace, newest first. `operator:global` sees every tenant's.
///
/// Scoped in Cluster 398.3: these rows carry `to_address`, `subject` and the
/// message body, and the query was global behind a per-workspace capability —
/// so any workspace admin could read every other tenant's outbound email.
pub async fn list_dead_mail(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListDeadMailQuery>,
) -> ApiResult<Json<Vec<DeadMail>>> {
    cap(&auth, TOKEN_ADMIN)?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(
        state.store.list_dead_mail(dlq_scope(&auth), limit).await?,
    ))
}

/// `POST /operator/mail/dead/{id}/requeue` — requeue a dead entry for retry
/// (`pending`, due now, `attempts` reset). `404` if no dead entry has that id.
pub async fn requeue_dead_mail(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, TOKEN_ADMIN)?;
    // Scoped like the list: another tenant's id is a 404, not a re-send of
    // their mail to their recipient.
    if state
        .store
        .requeue_dead_mail(dlq_scope(&auth), MailOutboxId(id))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
