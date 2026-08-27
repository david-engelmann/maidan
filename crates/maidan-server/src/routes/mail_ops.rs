//! Operator DLQ read for the durable mail outbox (Cluster 306): list
//! dead-lettered notification emails and requeue one for another delivery
//! attempt. Global + system-level, so gated on `token:admin`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
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

/// `GET /operator/mail/dead` — dead-lettered outbox entries, newest first.
pub async fn list_dead_mail(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListDeadMailQuery>,
) -> ApiResult<Json<Vec<DeadMail>>> {
    cap(&auth, TOKEN_ADMIN)?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(state.store.list_dead_mail(limit).await?))
}

/// `POST /operator/mail/dead/{id}/requeue` — requeue a dead entry for retry
/// (`pending`, due now, `attempts` reset). `404` if no dead entry has that id.
pub async fn requeue_dead_mail(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, TOKEN_ADMIN)?;
    if state.store.requeue_dead_mail(MailOutboxId(id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
