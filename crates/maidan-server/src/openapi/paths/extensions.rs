//! OpenAPI stubs for routes wired in `app.rs` but missing from `api.rs` (Cluster 77).

use uuid::Uuid;

use crate::dto::*;
use crate::thread_context::WorkspaceContext;
use maidan_types::*;

#[utoipa::path(
    post,
    path = "/workspaces/{id}/purge",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Messages purged"))
)]
pub fn purge_workspace() {}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/export",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Workspace content bundle (JSON)"))
)]
pub fn export_workspace() {}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/usage",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = WorkspaceUsage))
)]
pub fn get_workspace_usage() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/audit",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ListAuditQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<AuditEvent>))
)]
pub fn list_workspace_audit() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/context",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        WorkspaceContextQuery,
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, body = WorkspaceContext))
)]
pub fn get_workspace_context() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/outbox/quarantined",
    tag = "workspaces",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Quarantined outbox rows"))
)]
pub fn list_quarantined_outbox() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/outbox/{oid}/replay",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("oid" = Uuid, Path, description = "Outbox row id"),
    ),
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Replay accepted"))
)]
pub fn replay_quarantined_outbox() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/apps",
    tag = "apps",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 201, description = "App registered"))
)]
pub fn register_app() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/apps",
    tag = "apps",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Installed apps"))
)]
pub fn list_apps() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/apps/{app_id}/install",
    tag = "apps",
    security(("bearerAuth" = [])),
    responses((status = 201, description = "App installed"))
)]
pub fn install_app() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/apps/{app_id}/oauth/authorize",
    tag = "apps",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Authorization URL or redirect"))
)]
pub fn authorize_app_install() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/app-installations",
    tag = "apps",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Installations"))
)]
pub fn list_app_installations() {}

#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/app-installations/{iid}",
    tag = "apps",
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Revoked"))
)]
pub fn revoke_app_installation() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/app-installations/{iid}/tokens",
    tag = "apps",
    security(("bearerAuth" = [])),
    responses((status = 201, body = MintApiTokenResponse))
)]
pub fn mint_app_token() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/dm",
    tag = "dm",
    security(("bearerAuth" = [])),
    responses((status = 201, description = "DM conversation"))
)]
pub fn open_dm_conversation() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/dm",
    tag = "dm",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "DM conversations"))
)]
pub fn list_dm_conversations() {}

#[utoipa::path(
    get,
    path = "/dm/{id}",
    tag = "dm",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "DM conversation"))
)]
pub fn get_dm_conversation() {}

#[utoipa::path(
    post,
    path = "/dm/{id}/messages",
    tag = "dm",
    security(("bearerAuth" = [])),
    responses((status = 201, body = Message))
)]
pub fn post_dm_message() {}

#[utoipa::path(
    get,
    path = "/dm/{id}/messages",
    tag = "dm",
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<Message>))
)]
pub fn list_dm_messages() {}

#[utoipa::path(
    delete,
    path = "/messages/{id}/purge",
    tag = "messages",
    security(("bearerAuth" = [])),
    responses((status = 204, description = "Purged"))
)]
pub fn purge_message() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/automation/dlq",
    tag = "automation",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Dead-letter deliveries"))
)]
pub fn list_automation_dlq() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/automation/deliveries",
    tag = "automation",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Automation deliveries"))
)]
pub fn list_automation_deliveries() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/automation/deliveries/{did}",
    tag = "automation",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Delivery row"))
)]
pub fn get_automation_delivery() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/automation/deliveries/{did}/replay",
    tag = "automation",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Replay enqueued"))
)]
pub fn replay_automation_delivery() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/deliveries",
    tag = "operator",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Webhook + automation deliveries"))
)]
pub fn list_unified_deliveries() {}

#[utoipa::path(
    get,
    path = "/workspaces/{wid}/deliveries/{did}",
    tag = "operator",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Delivery row"))
)]
pub fn get_unified_delivery() {}

#[utoipa::path(
    post,
    path = "/workspaces/{wid}/deliveries/{did}/replay",
    tag = "operator",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Replay enqueued"))
)]
pub fn replay_unified_delivery() {}

#[utoipa::path(
    get,
    path = "/operator/audit",
    tag = "operator",
    params(("limit" = Option<i64>, Query, description = "Max events (default 50, clamped 1..=500)")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Audit events across all workspaces"),
        (status = 403, description = "Missing audit:read-global capability"),
    )
)]
pub fn list_global_audit() {}

#[utoipa::path(
    post,
    path = "/operator/reindex-embeddings",
    tag = "operator",
    request_body = crate::reindex_ops::StartReindexEmbeddings,
    security(("bearerAuth" = [])),
    responses(
        (status = 202, description = "Reindex job accepted", body = maidan_types::ReindexJob),
    )
)]
pub fn start_reindex_embeddings() {}

#[utoipa::path(
    get,
    path = "/operator/reindex-embeddings/{job_id}",
    tag = "operator",
    params(("job_id" = Uuid, Path, description = "Reindex job id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Job status", body = maidan_types::ReindexJob),
        (status = 404, description = "Unknown job"),
    )
)]
pub fn get_reindex_embeddings_job() {}
