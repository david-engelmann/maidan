//! OIDC and browser session routes (no bearer on login/callback).

use uuid::Uuid;

use crate::dto::{
    ListEventsQuery, MintApiTokenResponse, OidcCallbackQuery, OidcLoginQuery, SessionResponse,
};
use crate::error::ProblemDetails;
use maidan_types::StoredEvent;

#[utoipa::path(
    get,
    path = "/auth/oidc/login",
    tag = "auth",
    params(OidcLoginQuery),
    responses(
        (status = 307, description = "Redirect to IdP (or mock callback when MAIDAN_OIDC_MOCK=1)"),
        (status = 403, body = ProblemDetails, description = "OIDC disabled"),
        (status = 404, body = ProblemDetails, description = "Workspace not found"),
    )
)]
pub fn oidc_login() {}

#[utoipa::path(
    get,
    path = "/auth/oidc/callback",
    tag = "auth",
    params(OidcCallbackQuery),
    responses(
        (status = 307, description = "Redirect after session cookie is set"),
        (status = 400, body = ProblemDetails),
        (status = 403, body = ProblemDetails),
    )
)]
pub fn oidc_callback() {}

#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    responses(
        (status = 307, description = "Redirect to /ui/ or IdP end-session when configured"),
        (status = 403, body = ProblemDetails, description = "OIDC disabled"),
    )
)]
pub fn oidc_logout() {}

#[utoipa::path(
    get,
    path = "/auth/session",
    tag = "auth",
    security(("sessionCookie" = [])),
    responses(
        (status = 200, body = SessionResponse),
        (status = 401, body = ProblemDetails),
        (status = 403, body = ProblemDetails, description = "OIDC disabled"),
    )
)]
pub fn get_auth_session() {}

#[utoipa::path(
    post,
    path = "/auth/session/mint",
    tag = "auth",
    security(("sessionCookie" = [])),
    responses(
        (status = 201, body = MintApiTokenResponse, description = "First token:admin in workspace"),
        (status = 401, body = ProblemDetails),
        (status = 403, body = ProblemDetails),
    )
)]
pub fn mint_auth_session_token() {}

#[utoipa::path(
    get,
    path = "/ui/api/workspaces/{wid}/events",
    tag = "auth",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ListEventsQuery,
    ),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<StoredEvent>),
        (status = 401, body = ProblemDetails),
    )
)]
pub fn ui_list_events() {}
