//! Operator HTTP API for automation delivery queue (Cluster 68.0).

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_store::AutomationDeliveryFilter;
use maidan_types::{AutomationDelivery, NewAuditEvent, WorkspaceId};
use serde::Deserialize;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

#[derive(Debug, Default, Deserialize)]
pub struct ListAutomationDeliveriesQuery {
    #[serde(default)]
    pub quarantined: bool,
    #[serde(default)]
    pub delivered: bool,
    #[serde(default = "default_list_limit")]
    pub limit: i64,
}

fn default_list_limit() -> i64 {
    50
}

fn parse_status_filter(q: &ListAutomationDeliveriesQuery) -> AutomationDeliveryFilter {
    if q.quarantined {
        AutomationDeliveryFilter::DeadLetter
    } else if q.delivered {
        AutomationDeliveryFilter::Delivered
    } else {
        AutomationDeliveryFilter::Pending
    }
}

pub async fn list_quarantined_automation_deliveries(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<Uuid>,
) -> ApiResult<Json<Vec<AutomationDelivery>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let limit = 50;
    let rows = state
        .store
        .list_automation_deliveries(workspace_id, AutomationDeliveryFilter::DeadLetter, limit)
        .await?;
    Ok(Json(rows))
}

pub async fn list_automation_deliveries(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<Uuid>,
    Query(q): Query<ListAutomationDeliveriesQuery>,
) -> ApiResult<Json<Vec<AutomationDelivery>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let limit = q.limit.clamp(1, 500);
    let rows = state
        .store
        .list_automation_deliveries(workspace_id, parse_status_filter(&q), limit)
        .await?;
    Ok(Json(rows))
}

pub async fn replay_automation_delivery(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, delivery_id)): Path<(Uuid, i64)>,
) -> ApiResult<Json<AutomationDelivery>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let row = state
        .store
        .replay_automation_delivery(delivery_id, workspace_id)
        .await?;
    let actor_id = if auth.bypass {
        None
    } else {
        Some(auth.member_id)
    };
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id,
            action: "automation_delivery.replay".into(),
            target_kind: Some("automation_delivery".into()),
            target_id: None,
            metadata: serde_json::json!({
                "workspace_id": workspace_id.0,
                "delivery_id": delivery_id,
            }),
        })
        .await?;
    Ok(Json(row))
}

pub async fn get_automation_delivery(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, delivery_id)): Path<(Uuid, i64)>,
) -> ApiResult<Json<AutomationDelivery>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state
            .store
            .get_automation_delivery(delivery_id, workspace_id)
            .await?,
    ))
}
