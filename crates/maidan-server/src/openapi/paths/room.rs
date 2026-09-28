//! OpenAPI stubs for room discovery, handles, named capability sets, and
//! holder-side token attenuation.

use crate::dto::{
    AttenuateToken, CapabilitySetView, DelegateToken, DelegateTokenResponse, MintApiTokenResponse,
    SetWorkspaceHandle,
};
use crate::error::ProblemDetails;
use crate::openapi::responses::*;
use maidan_types::{RoomCard, RoomDiscovery, WorkspaceHandle};
use uuid::Uuid;

/// Discover this server's rooms
#[utoipa::path(
    get,
    path = "/.well-known/maidan-room",
    tag = "federation",
    security(()),
    responses(
        (status = 200, body = RoomDiscovery),
    )
)]
pub fn well_known_room() {}

/// Get a workspace's room card
#[utoipa::path(
    get,
    path = "/workspaces/{id}/room",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = RoomCard),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_workspace_room() {}

/// Get a workspace's handle
#[utoipa::path(
    get,
    path = "/workspaces/{id}/handle",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WorkspaceHandle),
        (status = 403, response = Forbidden),
        (status = 404, description = "No handle set", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn get_workspace_handle() {}

/// Set a workspace's handle
#[utoipa::path(
    put,
    path = "/workspaces/{id}/handle",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = SetWorkspaceHandle,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WorkspaceHandle),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 413, response = PayloadTooLarge),
    )
)]
pub fn set_workspace_handle() {}

/// List named capability sets
#[utoipa::path(
    get,
    path = "/capability-sets",
    tag = "tokens",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<CapabilitySetView>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_capability_sets() {}

/// Derive a weaker token from the caller's
#[utoipa::path(
    post,
    path = "/tokens/attenuate",
    tag = "tokens",
    request_body = AttenuateToken,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintApiTokenResponse),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 413, response = PayloadTooLarge),
    )
)]
pub fn attenuate_api_token() {}

/// Exchange a delegation grant for a token
#[utoipa::path(
    post,
    path = "/tokens/delegate",
    tag = "tokens",
    request_body = DelegateToken,
    responses(
        (status = 201, description = "Short-lived token bound to the delegation grant", body = DelegateTokenResponse),
        (status = 400, description = "Invalid capability scope or expiry", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 401, description = "Grant expired or revoked", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Wrong delegate or workspace", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, response = NotFound),
        (status = 413, response = PayloadTooLarge),
    ),
    security(("bearerAuth" = ["workspace:read"]))
)]
pub fn delegate_api_token() {}
