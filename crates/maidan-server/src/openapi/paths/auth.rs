//! OIDC and browser session routes (no bearer on login/callback).

use crate::openapi::responses::*;
use uuid::Uuid;

use crate::dto::{
    CreateChannel, CreateMessage, CreateThread, ListAuditQuery, ListEventsQuery,
    ListMessageEditsQuery, ListMessagesQuery, ListThreadsQuery, MintApiTokenResponse,
    OidcCallbackQuery, OidcLoginQuery, PeerResponse, RenameWorkspace, SearchQuery, SessionResponse,
};
use crate::error::ProblemDetails;
use crate::openapi::schemas::SearchHit;
use maidan_types::{AuditEvent, Channel, Message, MessageEdit, StoredEvent, Thread, Workspace};

/// Start an OIDC login
#[utoipa::path(
    get,
    path = "/auth/oidc/login",
    tag = "auth",
    params(OidcLoginQuery),
    security(()),
    responses(
        (status = 307, description = "Redirect to IdP (or mock callback when MAIDAN_OIDC_MOCK=1)"),
        (status = 403, description = "OIDC disabled", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "Workspace not found", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn oidc_login() {}

/// Finish an OIDC login
#[utoipa::path(
    get,
    path = "/auth/oidc/callback",
    tag = "auth",
    params(OidcCallbackQuery),
    security(()),
    responses(
        (status = 307, description = "Redirect after session cookie is set"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn oidc_callback() {}

/// Log out of the browser session
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    security(()),
    responses(
        (status = 303, description = "Redirect to /ui/, or to the IdP end-session page for an OIDC session when configured"),
        (status = 403, description = "Browser sessions are not configured", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, response = NotFound),
    )
)]
pub fn oidc_logout() {}

/// Get the browser session
#[utoipa::path(
    get,
    path = "/auth/session",
    tag = "auth",
    security(("sessionCookie" = [])),
    responses(
        (status = 200, body = SessionResponse),
        (status = 403, description = "OIDC disabled", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, response = NotFound),
    )
)]
pub fn get_auth_session() {}

/// Mint the first admin token from the browser session
#[utoipa::path(
    post,
    path = "/auth/session/mint",
    tag = "auth",
    security(("sessionCookie" = [])),
    responses(
        (status = 201, body = MintApiTokenResponse, description = "First token:admin in workspace"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn mint_auth_session_token() {}

/// Exchange a bearer for a browser session
///
/// Sets the `HttpOnly; SameSite=Lax` `maidan_session` cookie for a session
/// holding the bearer's authority, so a page need not keep the token. Every
/// request on the session resolves the token again, so revoking, rotating or
/// expiring it ends the session. The bearer must be sent in `Authorization`;
/// a session cannot make another.
#[utoipa::path(
    post,
    path = "/auth/session/from-token",
    tag = "auth",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = SessionResponse, description = "Session created; the cookie is in `Set-Cookie`",
            headers(("Set-Cookie" = String, description = "The `maidan_session` cookie"))),
        (status = 401, description = "No bearer in `Authorization` (a session cannot make another), or the credential is not an API token", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, response = Forbidden),
        (status = 404, description = "Browser sessions are not configured (no `MAIDAN_SESSION_SECRET`)", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn session_from_token() {}

/// Exchange an OAuth authorization code for an access token
///
/// The OAuth 2.1 token endpoint. The form carries the code, the redirect URI
/// it was issued for, the client id, and the PKCE verifier; confidential
/// clients also send their secret. The code is validated before it is
/// consumed, so a wrong guess cannot burn the real client's code. The
/// minted token is capability-scoped from the member's grant and never
/// carries `approval:grant`.
#[utoipa::path(
    post,
    path = "/oauth/token",
    tag = "auth",
    request_body(
        content = crate::oauth::token::OAuthTokenRequest,
        content_type = "application/x-www-form-urlencoded",
    ),
    security(()),
    responses(
        (status = 200, body = crate::oauth::token::OAuthTokenResponse),
        (status = 400, response = BadRequest),
        (status = 401, response = Unauthorized),
    )
)]
pub fn oauth_token() {}

/// Start an OAuth authorization request
///
/// The OAuth 2.1 authorization endpoint. Validates the request (client,
/// redirect URI, PKCE challenge, resource indicator) and stores it pending
/// the member's consent decision, then redirects to the consent page. Never
/// issues a code on a GET.
#[utoipa::path(
    get,
    path = "/oauth/authorize",
    tag = "auth",
    params(
        ("client_id" = String, Query, description = "The client's id, or an HTTPS client metadata URL (CIMD)"),
        ("redirect_uri" = String, Query, description = "Where the code goes; must match a registered URI exactly"),
        ("code_challenge" = String, Query, description = "PKCE S256 challenge"),
        ("code_challenge_method" = String, Query, description = "Must be S256"),
        ("scope" = Option<String>, Query, description = "Space-delimited capabilities"),
        ("state" = String, Query, description = "Opaque client state, echoed back"),
        ("resource" = Option<String>, Query, description = "RFC 8707 resource indicator"),
    ),
    security(()),
    responses(
        (status = 307, description = "Redirect to the consent page, or to the client with an error"),
        (status = 400, response = BadRequest),
        (status = 401, response = Unauthorized),
    )
)]
pub fn oauth_authorize() {}

/// List a workspace's events (console)
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
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, description = "Cursor too old; must_refetch", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn ui_list_events() {}

/// List a workspace's channels (console)
#[utoipa::path(
    get,
    path = "/ui/api/workspaces/{wid}/channels",
    tag = "auth",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<Channel>),
        (status = 403, response = Forbidden),
    )
)]
pub fn ui_list_channels() {}

/// Name a workspace (console)
#[utoipa::path(
    patch,
    path = "/ui/api/workspaces/{wid}",
    tag = "auth",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = RenameWorkspace,
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Workspace),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_rename_workspace() {}

/// Create a channel (console)
#[utoipa::path(
    post,
    path = "/ui/api/workspaces/{wid}/channels",
    tag = "auth",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = CreateChannel,
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 201, body = Channel),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn ui_create_channel() {}

/// Create a thread in a channel (console)
#[utoipa::path(
    post,
    path = "/ui/api/channels/{cid}/threads",
    tag = "auth",
    params(("cid" = Uuid, Path, description = "Channel id")),
    request_body = CreateThread,
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 201, body = Thread),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    )
)]
pub fn ui_create_thread() {}

/// Post a message to a thread (console)
#[utoipa::path(
    post,
    path = "/ui/api/threads/{tid}/messages",
    tag = "auth",
    params(("tid" = Uuid, Path, description = "Thread id")),
    request_body = CreateMessage,
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 201, body = Message),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    )
)]
pub fn ui_post_message() {}

/// List a channel's threads (console)
#[utoipa::path(
    get,
    path = "/ui/api/channels/{cid}/threads",
    tag = "auth",
    params(("cid" = Uuid, Path, description = "Channel id"), ListThreadsQuery),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<Thread>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_list_threads() {}

/// List a thread's messages (console)
#[utoipa::path(
    get,
    path = "/ui/api/threads/{tid}/messages",
    tag = "auth",
    params(
        ("tid" = Uuid, Path, description = "Thread id"),
        ListMessagesQuery,
    ),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<Message>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_list_messages() {}

/// Search a workspace's messages (console)
#[utoipa::path(
    get,
    path = "/ui/api/workspaces/{wid}/search",
    tag = "auth",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        SearchQuery,
    ),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<SearchHit>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_search_messages() {}

/// List a workspace's audit events (console)
#[utoipa::path(
    get,
    path = "/ui/api/workspaces/{wid}/audit",
    tag = "auth",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ListAuditQuery,
    ),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<AuditEvent>),
        (status = 403, response = Forbidden),
    )
)]
pub fn ui_list_audit() {}

/// List a workspace's federation peers (console)
#[utoipa::path(
    get,
    path = "/ui/api/workspaces/{wid}/peers",
    tag = "auth",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<PeerResponse>),
        (status = 403, response = Forbidden),
    )
)]
pub fn ui_list_peers() {}

/// List a message's edit history (console)
#[utoipa::path(
    get,
    path = "/ui/api/messages/{mid}/edits",
    tag = "auth",
    params(
        ("mid" = Uuid, Path, description = "Message id"),
        ListMessageEditsQuery,
    ),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<MessageEdit>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_list_message_edits() {}
