//! The grant management page (`docs/OAuth.md`, phase four).
//!
//! `GET /ui/oauth/grants` shows the signed-in member every client that
//! holds their consent in this workspace; each row's form revokes that
//! grant, which ends its whole lineage — refresh and access tokens
//! alike — through the same audited lineage revocation the revocation
//! endpoint uses.
//!
//! Like consent, this is a human surface: the session must be one the
//! person signed in to (`SessionContext::token` is `None`), because a
//! token-made session proves no more than the token behind it, and the
//! revoke POST must be same-origin.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
    Extension,
};
use maidan_types::OAuthGrantId;
use uuid::Uuid;

use super::consent::html_escape;
use crate::app::BOARD_UI_CSP;
use crate::error::ApiError;
use crate::extract::ApiPath;
use crate::session::{require_same_origin, SessionContext};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// A token-made session has no person behind it to own the consent
/// being managed; refuse it on both the page and the revoke.
fn require_human_session(session: &SessionContext) -> ApiResult<()> {
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "grant management needs the signed-in session, not a token".into(),
        ));
    }
    Ok(())
}

/// Render the member's live grants in the session's workspace. A client
/// shows by the name on its registry row (a metadata-document client
/// gets its row on first use at `/authorize`); a row without a recorded
/// name shows by client id alone.
pub async fn grants_page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionContext>,
) -> ApiResult<impl IntoResponse> {
    require_human_session(&session)?;
    let grants = state
        .store
        .list_oauth_grants(session.workspace_id, session.member_id)
        .await?;
    let mut rows = String::new();
    for grant in &grants {
        let client_name = match state
            .store
            .get_oauth_client_by_client_id(&grant.client_id)
            .await?
        {
            Some(client) => html_escape(&client.name),
            None => html_escape(&grant.client_id),
        };
        let scopes: String = grant
            .scope
            .iter()
            .map(|s| format!("<li><code>{}</code></li>", html_escape(s)))
            .collect::<Vec<_>>()
            .join("");
        rows.push_str(&format!(
            "<section><h2>{client_name}</h2><p><code>{client_id}</code></p><ul>{scopes}</ul><form method=\"post\" action=\"/ui/api/oauth/grants/{grant_id}/revoke\"><button type=\"submit\">Revoke</button></form></section>",
            client_id = html_escape(&grant.client_id),
            grant_id = grant.id.0,
        ));
    }
    if rows.is_empty() {
        rows.push_str("<p>No apps hold your consent in this workspace.</p>");
    }
    let body = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>Connected apps</title></head>
<body>
<h1>Connected apps</h1>
<p>These clients hold your consent in this workspace. Revoking one ends its access tokens and refresh tokens at once.</p>
{rows}
</body>
</html>"#
    );
    Ok((
        [(axum::http::header::CONTENT_SECURITY_POLICY, BOARD_UI_CSP)],
        Html(body),
    ))
}

/// Revoke one of the session member's grants. Same-origin POST on the
/// signed-in session only. A grant of another member, or of this member
/// in another workspace, is a 404.
pub async fn revoke_grant(
    State(state): State<AppState>,
    Extension(session): Extension<SessionContext>,
    headers: HeaderMap,
    ApiPath(grant_id): ApiPath<Uuid>,
) -> ApiResult<impl IntoResponse> {
    require_human_session(&session)?;
    require_same_origin(&headers)?;
    let grant = state
        .store
        .get_oauth_grant(OAuthGrantId(grant_id))
        .await?
        .filter(|grant| {
            grant.workspace_id == session.workspace_id && grant.member_id == session.member_id
        })
        .ok_or(ApiError::NotFound)?;
    state
        .store
        .revoke_oauth_lineage_audited(
            grant.lineage_id,
            super::token::lineage_audit_as(
                "member_revocation",
                grant.client_id.clone(),
                Some(session.member_id),
            ),
        )
        .await?;
    Ok((StatusCode::SEE_OTHER, Redirect::to("/ui/oauth/grants")))
}
