//! A workspace's own GitHub, Slack and Linear connections: list, set, remove.
//!
//! The token is the workspace's credential for the service, kept as a
//! reserved workspace secret ([`maidan_types::ConnectionSurface::secret_name`])
//! that no generic secret route or tool can touch. It is checked with the
//! provider before it is stored, and no route, log line or audit row ever
//! carries a token or an inbound secret.
//!
//! Only the workspace's admin may see or change a connection: a bearer needs
//! `token:admin`, and a person signed in to the console must hold it on a live
//! token or be able to mint the first one, as for the egress allowlist. A
//! connection decides where the workspace's results and links reach, so it is
//! policy as much as the allowlist is.

use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::Utc;
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::{
    AuditScope, ConnectionSurface, MemberKind, NewAuditEvent, NewSecret, WorkspaceId,
};

use super::{cap, ensure_workspace, ApiResult};
use crate::connections::{ConnectionError, Source, StoredConnection, VerifyError};
use crate::dto::{Connection, ConnectionInbound, ConnectionSource, SetConnection};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::session::SessionContext;
use crate::state::AppState;

/// `GET /workspaces/:wid/connections`: the three surfaces, in order, and who
/// serves the workspace on each.
pub async fn list_connections(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<Connection>>> {
    let workspace_id = WorkspaceId(wid);
    bearer_admin(&auth, workspace_id)?;
    list(&state, workspace_id).await
}

/// `PUT /workspaces/:wid/connections/:surface`: connect the workspace's own
/// account, after the provider accepts the token.
pub async fn set_connection(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((wid, surface)): ApiPath<(uuid::Uuid, String)>,
    ApiJson(body): ApiJson<SetConnection>,
) -> ApiResult<Json<Connection>> {
    let workspace_id = WorkspaceId(wid);
    bearer_admin(&auth, workspace_id)?;
    set(&state, &auth, workspace_id, &surface, body).await
}

/// `DELETE /workspaces/:wid/connections/:surface`: forget the workspace's own
/// connection. `204` whether or not it had one.
pub async fn clear_connection(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((wid, surface)): ApiPath<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    bearer_admin(&auth, workspace_id)?;
    clear(&state, &auth, workspace_id, &surface).await
}

/// `GET /ui/api/workspaces/:wid/connections`.
pub async fn ui_list_connections(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<Connection>>> {
    let workspace_id = WorkspaceId(wid);
    console_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
    )
    .await?;
    list(&state, workspace_id).await
}

/// `PUT /ui/api/workspaces/:wid/connections/:surface`.
pub async fn ui_set_connection(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath((wid, surface)): ApiPath<(uuid::Uuid, String)>,
    ApiJson(body): ApiJson<SetConnection>,
) -> ApiResult<Json<Connection>> {
    let workspace_id = WorkspaceId(wid);
    console_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
    )
    .await?;
    set(&state, &auth, workspace_id, &surface, body).await
}

/// `DELETE /ui/api/workspaces/:wid/connections/:surface`.
pub async fn ui_clear_connection(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath((wid, surface)): ApiPath<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    console_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
    )
    .await?;
    clear(&state, &auth, workspace_id, &surface).await
}

fn bearer_admin(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    cap(auth, TOKEN_ADMIN)?;
    ensure_workspace(auth, workspace_id)
}

/// Whether the caller of a console route may see and change connections. A
/// bearer is held to `token:admin`. A person signed in qualifies when they
/// are a human member who holds `token:admin` on a live token or may mint the
/// first one, so the session lends them nothing a token of theirs could not
/// do. An agent's sign-in never qualifies, and a session made from a token is
/// refused: that token's holder uses the bearer routes.
async fn console_admin(
    state: &AppState,
    auth: &AuthContext,
    session: Option<&SessionContext>,
    workspace_id: WorkspaceId,
) -> ApiResult<()> {
    let Some(session) = session else {
        return bearer_admin(auth, workspace_id);
    };
    ensure_workspace(auth, workspace_id)?;
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "a session made from a token cannot use connections here; that token's holder uses /workspaces/{wid}/connections".into(),
        ));
    }
    let member = state
        .store
        .get_member_in(workspace_id, session.member_id)
        .await?;
    if member.kind == MemberKind::Human
        && super::member::session_holds_or_may_mint(state, session, TOKEN_ADMIN).await?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "only a workspace admin can see or change the workspace's connections".into(),
        ))
    }
}

/// An unknown surface is a route that does not exist.
fn parse_surface(raw: &str) -> ApiResult<ConnectionSurface> {
    ConnectionSurface::parse(raw).ok_or(ApiError::NotFound)
}

/// A connection that cannot be read is the server's fault to tell the admin
/// about, not a reason to hide the surface.
fn connection_error(err: ConnectionError) -> ApiError {
    match err {
        ConnectionError::Store(err) => err.into(),
        err @ ConnectionError::Unreadable { .. } => ApiError::Internal(err.to_string()),
    }
}

/// Where a surface's deliveries to a workspace arrive. Relative to the server
/// when it knows no public origin.
fn inbound_url(state: &AppState, workspace_id: WorkspaceId, surface: ConnectionSurface) -> String {
    let path = match surface {
        ConnectionSurface::Github => "webhook",
        _ => "events",
    };
    let origin = state
        .public_origin
        .as_deref()
        .unwrap_or_default()
        .trim_end_matches('/');
    format!(
        "{origin}/workspaces/{}/connections/{surface}/{path}",
        workspace_id.0
    )
}

/// One surface's row.
async fn row(
    state: &AppState,
    workspace_id: WorkspaceId,
    surface: ConnectionSurface,
) -> ApiResult<Connection> {
    let inbound = |secret_set| {
        (surface != ConnectionSurface::Linear).then(|| ConnectionInbound {
            url: inbound_url(state, workspace_id, surface),
            secret_set,
        })
    };
    match state.stored_connection(workspace_id, surface).await {
        Ok(Some((_, stored))) => Ok(Connection {
            surface,
            source: Some(ConnectionSource::Workspace),
            account: Some(stored.account.clone()),
            connected_at: Some(stored.connected_at),
            inbound: inbound(stored.inbound_secret(surface).is_some()),
        }),
        Ok(None) => {
            let source = state
                .connection_source(workspace_id, surface)
                .await
                .map_err(connection_error)?;
            Ok(Connection {
                surface,
                source: source.map(|source| match source {
                    Source::Workspace => ConnectionSource::Workspace,
                    Source::Instance => ConnectionSource::Instance,
                }),
                account: None,
                connected_at: None,
                inbound: inbound(false),
            })
        }
        // Stored but unreadable: still the workspace's own, so it can be
        // replaced or removed, and the listing does not fail for it.
        Err(ConnectionError::Unreadable { .. }) => Ok(Connection {
            surface,
            source: Some(ConnectionSource::Workspace),
            account: None,
            connected_at: None,
            inbound: inbound(false),
        }),
        Err(err) => Err(connection_error(err)),
    }
}

async fn list(state: &AppState, workspace_id: WorkspaceId) -> ApiResult<Json<Vec<Connection>>> {
    let mut rows = Vec::with_capacity(ConnectionSurface::ALL.len());
    for surface in ConnectionSurface::ALL {
        rows.push(row(state, workspace_id, surface).await?);
    }
    Ok(Json(rows))
}

/// An inbound secret is 16 to 128 characters with no whitespace.
fn checked_secret(name: &str, value: &str) -> ApiResult<String> {
    let value = value.trim();
    if value.len() < 16 || value.len() > 128 || value.chars().any(char::is_whitespace) {
        return Err(ApiError::BadRequest(format!(
            "{name} is 16 to 128 characters with no spaces"
        )));
    }
    Ok(value.to_string())
}

async fn set(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    surface: &str,
    body: SetConnection,
) -> ApiResult<Json<Connection>> {
    let surface = parse_surface(surface)?;
    // Which inbound secret this surface takes, and none where it takes none.
    match surface {
        ConnectionSurface::Github if body.signing_secret.is_some() => {
            return Err(ApiError::BadRequest(
                "signing_secret is for Slack; GitHub takes webhook_secret".into(),
            ));
        }
        ConnectionSurface::Slack if body.webhook_secret.is_some() => {
            return Err(ApiError::BadRequest(
                "webhook_secret is for GitHub; Slack takes signing_secret".into(),
            ));
        }
        ConnectionSurface::Linear
            if body.webhook_secret.is_some() || body.signing_secret.is_some() =>
        {
            return Err(ApiError::BadRequest(
                "Linear takes a token only; nothing is sent in from it".into(),
            ));
        }
        _ => {}
    }
    let webhook_secret = body
        .webhook_secret
        .as_deref()
        .map(|v| checked_secret("webhook_secret", v))
        .transpose()?;
    let signing_secret = body
        .signing_secret
        .as_deref()
        .map(|v| checked_secret("signing_secret", v))
        .transpose()?;
    let token = body.token.as_deref().map(str::trim);
    if let Some(token) = token
        && (token.is_empty() || token.len() > 512 || token.chars().any(char::is_whitespace))
    {
        return Err(ApiError::BadRequest(
            "token is 1 to 512 characters with no spaces".into(),
        ));
    }
    // Not a failure of this request: the instance was set up without the key
    // that encrypts workspace secrets, so it has nowhere to keep a connection.
    let key = state.federation.encryption_key.as_deref().ok_or_else(|| {
        ApiError::Overloaded(
            "this instance cannot store a workspace's connection: storing one needs FEDERATION_ENCRYPTION_KEY on the instance".into(),
        )
    })?;
    let existing = match state.stored_connection(workspace_id, surface).await {
        Ok(existing) => existing.map(|(_, connection)| connection),
        // A connection that cannot be opened is replaced by a new token, and
        // cannot have its secret updated alone.
        Err(ConnectionError::Unreadable { .. }) if token.is_some() => None,
        Err(err) => return Err(connection_error(err)),
    };
    let connection = match (token, existing) {
        (Some(token), existing) => {
            let account = state
                .connection_factory
                .verify(surface, token)
                .await
                .map_err(|err| match err {
                    VerifyError::Refused(reason) => ApiError::BadRequest(format!(
                        "{} refused this token: {reason}",
                        surface.label()
                    )),
                    VerifyError::Unavailable(reason) => {
                        tracing::warn!(%surface, %reason, "connection check: provider unavailable");
                        ApiError::BadGateway(format!(
                            "{} could not be asked to check this token; nothing was stored",
                            surface.label()
                        ))
                    }
                })?;
            // Replacing a token keeps the inbound secret already stored, so
            // rotating a token does not silently stop verifying deliveries.
            StoredConnection {
                token: token.to_string(),
                account,
                webhook_secret: webhook_secret
                    .or_else(|| existing.as_ref().and_then(|e| e.webhook_secret.clone())),
                signing_secret: signing_secret
                    .or_else(|| existing.as_ref().and_then(|e| e.signing_secret.clone())),
                connected_at: Utc::now(),
            }
        }
        (None, Some(mut existing)) if webhook_secret.is_some() || signing_secret.is_some() => {
            existing.webhook_secret = webhook_secret.or(existing.webhook_secret);
            existing.signing_secret = signing_secret.or(existing.signing_secret);
            existing
        }
        (None, _) => {
            return Err(ApiError::BadRequest(format!(
                "token is required to connect {}",
                surface.label()
            )));
        }
    };
    let value_ciphertext = maidan_auth::encrypt_peer_secret(&connection.to_plaintext(surface), key)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let actor = auth.actor_id();
    let account = connection.account.clone();
    state
        .store
        .create_secret_audited(
            NewSecret {
                workspace_id,
                name: surface.secret_name().into(),
                value_ciphertext,
                created_by: auth.member_id(),
            },
            Box::new(move |secret| NewAuditEvent {
                scope: AuditScope::Workspace(secret.workspace_id),
                actor_id: Some(actor),
                action: "connection.set".into(),
                target_kind: Some("secret".into()),
                target_id: Some(secret.id.0),
                // The surface and the account, never the token or a secret.
                metadata: serde_json::json!({
                    "workspace_id": secret.workspace_id.0,
                    "surface": surface.as_str(),
                    "account": account,
                }),
            }),
        )
        .await?;
    // The next send builds its client from what was just stored.
    row(state, workspace_id, surface).await.map(Json)
}

async fn clear(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    surface: &str,
) -> ApiResult<StatusCode> {
    let surface = parse_surface(surface)?;
    state
        .store
        .delete_secret_audited(
            workspace_id,
            surface.secret_name(),
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id()),
                action: "connection.clear".into(),
                target_kind: Some("secret".into()),
                target_id: None,
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "surface": surface.as_str(),
                }),
            },
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
