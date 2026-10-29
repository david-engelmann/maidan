//! OIDC and browser session routes (no bearer on login/callback).

use crate::openapi::responses::*;
use uuid::Uuid;

use crate::dto::{
    ConfirmApprovalGate, CreateChannel, CreateMessage, CreateThread, ListAuditQuery,
    ListEventsQuery, ListMessageEditsQuery, ListMessagesQuery, ListThreadsQuery,
    MintApiTokenResponse, OidcCallbackQuery, OidcLoginQuery, OidcSignupForm, OidcSignupQuery,
    PeerResponse, RenameWorkspace, SearchQuery, SessionResponse, SessionWorkspaces,
    SetMemberDisplayName, ThreadTransitionsQuery,
};
use crate::error::ProblemDetails;
use crate::openapi::schemas::SearchHit;
use maidan_types::{
    ApprovalGate, AuditEvent, Channel, Member, Message, MessageEdit, StoredEvent, Thread,
    ThreadArtifact, ThreadReview, ThreadTransition, Workspace,
};

/// Start an OIDC login
///
/// With `workspace_id`, signs in to that workspace. The id is not looked up
/// before the redirect: an unknown id redirects like a real one and the
/// callback refuses it with the same 403 as a workspace the person has no
/// member in. Without it (the front door), the callback signs in to the
/// identity's most recently used workspace, adds `choose_workspace=1` to the
/// return path when there are more, and creates no session (adding
/// `no_workspace=1`) when there are none.
#[utoipa::path(
    get,
    path = "/auth/oidc/login",
    tag = "auth",
    params(OidcLoginQuery),
    security(()),
    responses(
        (status = 307, description = "Redirect to IdP (or mock callback when MAIDAN_OIDC_MOCK=1)"),
        (status = 400, response = BadRequest),
        (status = 403, description = "OIDC disabled", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn oidc_login() {}

/// Start an invited sign-up
///
/// Creates a workspace for someone who is not a member anywhere yet. The
/// `invite` names the waitlist invite the operator minted; without one, the
/// address the identity provider later verifies must sit in the instance's
/// sign-up domain list. The callback consumes the pending sign-up and
/// creates the workspace, its first member, the identity and the session in
/// one transaction. 404 unless sign-up is enabled on the instance.
#[utoipa::path(
    get,
    path = "/auth/oidc/signup",
    tag = "auth",
    params(OidcSignupQuery),
    security(()),
    responses(
        (status = 307, description = "Redirect to IdP (or mock callback when MAIDAN_OIDC_MOCK=1)"),
        (status = 400, response = BadRequest),
        (status = 403, description = "OIDC disabled, the invite is unknown, spent or expired, or sign-up here is by invite only", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "Sign-up is not enabled on this instance", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn oidc_signup() {}

/// Start a sign-up after the Turnstile check
///
/// The console's form POST once its Turnstile widget has a token. Same start
/// as the GET, with the token in the form body so it never sits in a URL.
/// Answers 303 to the identity provider.
#[utoipa::path(
    post,
    path = "/auth/oidc/signup",
    tag = "auth",
    request_body(content = OidcSignupForm, content_type = "application/x-www-form-urlencoded"),
    security(()),
    responses(
        (status = 303, description = "Redirect to IdP (or mock callback when MAIDAN_OIDC_MOCK=1)"),
        (status = 400, response = BadRequest),
        (status = 403, description = "OIDC disabled, the Turnstile check failed, the invite is unknown, spent or expired, or sign-up here is by invite only", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "Sign-up is not enabled on this instance", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn oidc_signup_form() {}

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

/// List the workspaces the signed-in person can switch to
///
/// The session's own workspace first, then every other workspace where the
/// identity this session signed in with (the same issuer and subject) is a
/// member, newest sign-in first, at most 200. A session made from a token
/// lists only its own workspace. A workspace whose member is SCIM-deactivated
/// or frozen is left out. To switch, sign in again at
/// `/auth/oidc/login?workspace_id=…`.
#[utoipa::path(
    get,
    path = "/auth/session/workspaces",
    tag = "auth",
    security(("sessionCookie" = [])),
    responses(
        (status = 200, body = SessionWorkspaces),
        (status = 401, response = Unauthorized),
    )
)]
pub fn list_auth_session_workspaces() {}

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

/// Confirm a model's request to accept an approval gate
///
/// The link `approval_decide` gave a person is spent here, by the signed-in
/// console session it is bound to. A bearer, and a session made from a token,
/// are refused: the model holding the token cannot finish it. The request must
/// come from the console page.
#[utoipa::path(
    post,
    path = "/auth/approval-confirmations/confirm",
    tag = "auth",
    request_body = ConfirmApprovalGate,
    security(("sessionCookie" = [])),
    responses(
        (status = 200, body = ApprovalGate, description = "The gate, now accepted"),
        (status = 403, description = "Not a signed-in person's session, not from the console page, or the caller asked for the gate", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such live link for this session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The gate was resolved before the link was spent", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn confirm_approval() {}

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

/// Exchange an OAuth authorization code or refresh token for tokens
///
/// The OAuth 2.1 token endpoint, advertised in the RFC 8414 document.
/// `grant_type=authorization_code` exchanges a code, its redirect URI and the
/// PKCE verifier for an access token and a refresh token. The code is
/// validated before it is consumed, so a wrong guess cannot burn the real
/// client's code, and a code presented again after use revokes its grant's
/// lineage. `grant_type=refresh_token` rotates: the presented refresh token is
/// marked used and a new pair is returned under the same lineage; a reused
/// refresh token revokes the whole lineage. `scope` may narrow a refresh,
/// never widen it. Every request names `resource`, the MCP endpoint, and the
/// access token is accepted on `/mcp/streamable` only (RFC 8707). The client
/// authenticates with HTTP Basic, its secret in the form, or, if public, PKCE
/// alone. Client errors are RFC 6749 section 5.2 errors, not problems. 404
/// until `MAIDAN_PUBLIC_ORIGIN` is set.
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
        (status = 400, description = "`invalid_request`, `invalid_grant`, `unauthorized_client`, `unsupported_grant_type`, `invalid_scope` or `invalid_target`", body = crate::oauth::error::OAuthErrorBody, content_type = "application/json"),
        (status = 401, description = "`invalid_client`; `WWW-Authenticate: Basic` when the client tried HTTP Basic", body = crate::oauth::error::OAuthErrorBody, content_type = "application/json"),
        (status = 404, response = NotFound),
    )
)]
pub fn oauth_token() {}

/// Revoke an OAuth access token or refresh token
///
/// RFC 7009 token revocation. The client authenticates as at the token
/// endpoint. A refresh token revokes its whole lineage (every grant, access
/// token and refresh token under it); an access token revokes that token
/// only. The answer is 200 with an empty body whether or not the token was
/// known, and a token issued to another client is left alone with the same
/// 200. Client errors are RFC 6749 section 5.2 errors, not problems. 404
/// until `MAIDAN_PUBLIC_ORIGIN` is set.
#[utoipa::path(
    post,
    path = "/oauth/revoke",
    tag = "auth",
    request_body(
        content = crate::oauth::revoke::OAuthRevokeRequest,
        content_type = "application/x-www-form-urlencoded",
    ),
    security(()),
    responses(
        (status = 200, description = "Revoked, or nothing of this client's to revoke"),
        (status = 400, description = "`invalid_request`", body = crate::oauth::error::OAuthErrorBody, content_type = "application/json"),
        (status = 401, description = "`invalid_client`; `WWW-Authenticate: Basic` when the client tried HTTP Basic", body = crate::oauth::error::OAuthErrorBody, content_type = "application/json"),
        (status = 404, response = NotFound),
    )
)]
pub fn oauth_revoke() {}

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
        ("response_type" = String, Query, description = "Must be `code`"),
        ("client_id" = String, Query, description = "The client's id, or an HTTPS client metadata URL (CIMD)"),
        ("redirect_uri" = String, Query, description = "Where the code goes; must match a registered URI exactly"),
        ("code_challenge" = String, Query, description = "PKCE S256 challenge"),
        ("code_challenge_method" = String, Query, description = "Must be S256"),
        ("scope" = Option<String>, Query, description = "Space-delimited capabilities"),
        ("state" = String, Query, description = "Opaque client state, echoed back"),
        ("resource" = String, Query, description = "RFC 8707 resource indicator: `<MAIDAN_PUBLIC_ORIGIN>/mcp/streamable`, the only resource tokens are issued for"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 307, description = "Redirect to the consent page, or to the client with an error"),
        (status = 400, response = BadRequest),
        (status = 401, response = Unauthorized),
        (status = 403, response = Forbidden),
        (status = 404, description = "The authorization server is off: no `MAIDAN_PUBLIC_ORIGIN`", body = ProblemDetails, content_type = "application/problem+json"),
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

/// List a thread's reviews (console)
///
/// The bearer tree's `GET /threads/{id}/reviews` for the console: each
/// reviewer's current review, so the approval card names who decided after a
/// reload.
#[utoipa::path(
    get,
    path = "/ui/api/threads/{tid}/reviews",
    tag = "auth",
    params(("tid" = Uuid, Path, description = "Thread id")),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<ThreadReview>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_list_reviews() {}

/// List a thread's linked artifacts (console)
///
/// The bearer tree's `GET /threads/{id}/artifacts` for the console: who linked
/// each artifact to the thread and when, so the approval card can say so and
/// flag an artifact the packet pinned that the thread no longer links.
#[utoipa::path(
    get,
    path = "/ui/api/threads/{tid}/artifacts",
    tag = "auth",
    params(("tid" = Uuid, Path, description = "Thread id")),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<ThreadArtifact>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_list_thread_artifacts() {}

/// List a thread's state transitions (console)
///
/// The bearer tree's `GET /threads/{id}/transitions` for the console: who
/// moved the task into review, so the approval card can name them.
#[utoipa::path(
    get,
    path = "/ui/api/threads/{tid}/transitions",
    tag = "auth",
    params(("tid" = Uuid, Path, description = "Thread id"), ThreadTransitionsQuery),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Vec<ThreadTransition>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_list_thread_transitions() {}

/// Get a member (console)
///
/// The bearer tree's `GET /members/{id}` for the console, so the session
/// panel can show the signed-in member's own display name.
#[utoipa::path(
    get,
    path = "/ui/api/members/{id}",
    tag = "auth",
    params(("id" = Uuid, Path, description = "Member id")),
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Member),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_get_member() {}

/// Set your own display name (console)
///
/// The bearer tree's `PATCH /members/{id}`. A blank name clears it. The name
/// is personal state, so the caller must be that member.
#[utoipa::path(
    patch,
    path = "/ui/api/members/{id}",
    tag = "auth",
    params(("id" = Uuid, Path, description = "Member id")),
    request_body = SetMemberDisplayName,
    security(
        ("bearerAuth" = []),
        ("sessionCookie" = []),
    ),
    responses(
        (status = 200, body = Member),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn ui_set_member_display_name() {}

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
