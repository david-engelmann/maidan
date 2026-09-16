//! Operator DLQ for the durable projector egress (Cluster 377.4): list
//! dead-lettered Slack/GitHub deliveries and requeue one for another attempt.
//! Global + system-level, so gated on `token:admin` — the mail DLQ shape
//! (Cluster 306).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::{DeadEgress, EgressOutboxId};
use serde::Deserialize;

use super::{cap, ApiResult};
use crate::error::ApiError;
use crate::state::AppState;

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Deserialize)]
pub struct ListDeadEgressQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

/// `GET /operator/egress/dead` — dead-lettered projector deliveries for **the
/// caller's workspace**, newest first: what failed, where it was going, and the
/// surface's own last error.
///
/// Scoped in Cluster 397.4. `token:admin` is minted per workspace, but this
/// query was global, so one tenant's admin could read every other tenant's
/// Slack channel ids, GitHub repositories and delivery errors — and then
/// requeue into them.
pub async fn list_dead_egress(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListDeadEgressQuery>,
) -> ApiResult<Json<Vec<DeadEgress>>> {
    cap(&auth, TOKEN_ADMIN)?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(
        state
            .store
            .list_dead_egress(auth.workspace_id, limit)
            .await?,
    ))
}

/// `POST /operator/egress/dead/{id}/requeue` — requeue a dead delivery
/// (`pending`, due now, `attempts` reset). `404` if no dead entry has that id.
///
/// For a delivery that dead-lettered because its link was disabled (Cluster
/// 377.3), fix the credential and **re-link** first: re-linking clears
/// `disabled_at`, and a requeue on a still-disabled link just fails the same way
/// again.
pub async fn requeue_dead_egress(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, TOKEN_ADMIN)?;
    // Scoped like the list: another tenant's id is a 404, not a re-send into
    // their channel.
    if state
        .store
        .requeue_dead_egress(auth.workspace_id, EgressOutboxId(id))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
