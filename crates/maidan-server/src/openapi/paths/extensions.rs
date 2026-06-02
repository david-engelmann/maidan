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
