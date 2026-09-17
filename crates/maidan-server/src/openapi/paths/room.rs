//! OpenAPI stubs for room discovery, handles, named capability sets, and
//! holder-side token attenuation.

use crate::dto::{AttenuateToken, CapabilitySetView, MintApiTokenResponse, SetWorkspaceHandle};
use maidan_types::{RoomCard, RoomDiscovery, WorkspaceHandle};
use uuid::Uuid;

#[utoipa::path(
    get,
    path = "/.well-known/maidan-room",
    tag = "federation",
    responses((status = 200, body = RoomDiscovery))
)]
pub fn well_known_room() {}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/room",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses((status = 200, body = RoomCard))
)]
pub fn get_workspace_room() {}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/handle",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WorkspaceHandle),
        (status = 404, description = "No handle set")
    )
)]
pub fn get_workspace_handle() {}

#[utoipa::path(
    put,
    path = "/workspaces/{id}/handle",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = SetWorkspaceHandle,
    security(("bearerAuth" = [])),
    responses((status = 200, body = WorkspaceHandle))
)]
pub fn set_workspace_handle() {}

#[utoipa::path(
    get,
    path = "/capability-sets",
    tag = "tokens",
    security(("bearerAuth" = [])),
    responses((status = 200, body = Vec<CapabilitySetView>))
)]
pub fn list_capability_sets() {}

#[utoipa::path(
    post,
    path = "/tokens/attenuate",
    tag = "tokens",
    request_body = AttenuateToken,
    security(("bearerAuth" = [])),
    responses((status = 201, body = MintApiTokenResponse))
)]
pub fn attenuate_api_token() {}
