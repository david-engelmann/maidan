//! Unified operator HTTP API for webhook + automation deliveries (Cluster 80.0).

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_store::AutomationDeliveryFilter;
use maidan_types::{NewAuditEvent, OperatorDelivery, WorkspaceId};
use serde::Deserialize;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Default, Deserialize)]
pub struct ListDeliveriesQuery {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub quarantined: bool,
    #[serde(default)]
    pub delivered: bool,
    #[serde(default = "default_list_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub struct DeliveryKindQuery {
    pub kind: String,
}

fn default_list_limit() -> i64 {
    50
}

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

fn parse_status_filter(q: &ListDeliveriesQuery) -> AutomationDeliveryFilter {
    if q.quarantined {
        AutomationDeliveryFilter::DeadLetter
    } else if q.delivered {
        AutomationDeliveryFilter::Delivered
    } else {
        AutomationDeliveryFilter::Pending
    }
}

fn include_webhook(kind: &Option<String>) -> bool {
    kind.as_deref().is_none_or(|k| k == "webhook" || k == "all")
}

fn include_automation(kind: &Option<String>) -> bool {
    kind.as_deref()
        .is_none_or(|k| k == "automation" || k == "all")
}

fn parse_delivery_kind(kind: &str) -> ApiResult<&'static str> {
    match kind {
        "webhook" => Ok("webhook"),
        "automation" => Ok("automation"),
        other => Err(ApiError::BadRequest(format!(
            "kind must be webhook or automation, got {other}"
        ))),
    }
}

pub async fn list_deliveries(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<Uuid>,
    Query(q): Query<ListDeliveriesQuery>,
) -> ApiResult<Json<Vec<OperatorDelivery>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    if let Some(kind) = &q.kind {
        if !matches!(kind.as_str(), "webhook" | "automation" | "all") {
            return Err(ApiError::BadRequest(
                "kind must be webhook, automation, or all".into(),
            ));
        }
    }
    let limit = q.limit.clamp(1, 500);
    let filter = parse_status_filter(&q);
    let mut rows = Vec::new();
    if include_webhook(&q.kind) {
        let webhook = state
            .store
            .list_webhook_deliveries(workspace_id, filter, limit)
            .await?;
        rows.extend(webhook.into_iter().map(OperatorDelivery::Webhook));
    }
    if include_automation(&q.kind) {
        let automation = state
            .store
            .list_automation_deliveries(workspace_id, filter, limit)
            .await?;
        rows.extend(automation.into_iter().map(OperatorDelivery::Automation));
    }
    rows.sort_by_key(|row| std::cmp::Reverse(delivery_sort_key(row)));
    rows.truncate(limit as usize);
    Ok(Json(rows))
}

fn delivery_sort_key(row: &OperatorDelivery) -> (chrono::DateTime<chrono::Utc>, i64) {
    match row {
        OperatorDelivery::Automation(d) => (d.created_at, d.id),
        OperatorDelivery::Webhook(d) => (d.created_at, d.id),
    }
}

pub async fn get_delivery(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, delivery_id)): Path<(Uuid, i64)>,
    Query(q): Query<DeliveryKindQuery>,
) -> ApiResult<Json<OperatorDelivery>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    match parse_delivery_kind(&q.kind)? {
        "webhook" => {
            let row = state
                .store
                .get_webhook_delivery(delivery_id, workspace_id)
                .await?;
            Ok(Json(OperatorDelivery::Webhook(row)))
        }
        "automation" => {
            let row = state
                .store
                .get_automation_delivery(delivery_id, workspace_id)
                .await?;
            Ok(Json(OperatorDelivery::Automation(row)))
        }
        // parse_delivery_kind only yields webhook|automation; treat anything
        // else as an internal invariant violation rather than panicking.
        other => Err(ApiError::Internal(format!(
            "unexpected delivery kind: {other}"
        ))),
    }
}

pub async fn replay_delivery(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, delivery_id)): Path<(Uuid, i64)>,
    Query(q): Query<DeliveryKindQuery>,
) -> ApiResult<Json<OperatorDelivery>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let kind = parse_delivery_kind(&q.kind)?;
    let row = match kind {
        "webhook" => OperatorDelivery::Webhook(
            state
                .store
                .replay_webhook_delivery(delivery_id, workspace_id)
                .await?,
        ),
        "automation" => OperatorDelivery::Automation(
            state
                .store
                .replay_automation_delivery(delivery_id, workspace_id)
                .await?,
        ),
        other => {
            return Err(ApiError::Internal(format!(
                "unexpected delivery kind: {other}"
            )))
        }
    };
    let actor_id = if auth.bypass {
        None
    } else {
        Some(auth.member_id)
    };
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id,
            action: "delivery.replay".into(),
            target_kind: Some("delivery".into()),
            target_id: None,
            metadata: serde_json::json!({
                "workspace_id": workspace_id.0,
                "delivery_id": delivery_id,
                "kind": kind,
            }),
        })
        .await?;
    Ok(Json(row))
}
