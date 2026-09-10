//! SCIM 2.0 user provisioning (Cluster 366, Wave 1 #14, SCIM-as-OIDC-P3). A
//! minimal RFC 7643/7644 endpoint so an IdP (Okta / Azure AD / …) can provision
//! and deprovision Maidan members: `ServiceProviderConfig` + `/Users` create /
//! read / list (with the `userName eq` filter) / replace / patch-active / delete.
//! A SCIM User maps to a member (`userName` = handle, `id` = member id); the
//! `maidan_scim_users` link tracks `externalId` + `active`. Deactivation
//! (`active=false`) and delete revoke the member's API tokens.
//!
//! Scoped to the caller's workspace and gated on `token:admin`. These routes are
//! intentionally outside the OpenAPI doc + capability-map (like `/mcp`) — SCIM has
//! its own schema and error envelope; auth is enforced inline here.

use axum::extract::{Path, RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::{Member, MemberKind, NewMember, ScimUser, WorkspaceId};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
const SPC_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";

/// A SCIM JSON response with the `application/scim+json` content type.
fn scim_response(status: StatusCode, value: Value) -> Response {
    let body = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
    (
        status,
        [(header::CONTENT_TYPE, "application/scim+json")],
        body,
    )
        .into_response()
}

/// A SCIM error envelope (RFC 7644 §3.12).
fn scim_error(status: StatusCode, detail: &str) -> Response {
    scim_response(
        status,
        json!({
            "schemas": [ERROR_SCHEMA],
            "status": status.as_u16().to_string(),
            "detail": detail,
        }),
    )
}

/// Require `token:admin` (bypass callers pass). Returns `Some(<403 error>)` when
/// the caller is not authorized, `None` when it may proceed.
fn require_admin(auth: &AuthContext) -> Option<Response> {
    if auth.bypass || auth.has_capability(TOKEN_ADMIN) {
        None
    } else {
        Some(scim_error(
            StatusCode::FORBIDDEN,
            "SCIM provisioning requires a token:admin bearer",
        ))
    }
}

#[derive(Deserialize)]
struct ScimName {
    formatted: Option<String>,
    #[serde(rename = "givenName")]
    given_name: Option<String>,
    #[serde(rename = "familyName")]
    family_name: Option<String>,
}

#[derive(Deserialize)]
struct ScimEmail {
    value: Option<String>,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
struct ScimUserInput {
    #[serde(rename = "userName")]
    user_name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    name: Option<ScimName>,
    #[serde(rename = "externalId")]
    external_id: Option<String>,
    #[serde(default = "default_true")]
    active: bool,
    #[serde(default)]
    emails: Vec<ScimEmail>,
}

fn default_true() -> bool {
    true
}

impl ScimUserInput {
    /// The member display name from the SCIM fields, best-effort.
    fn resolved_display_name(&self) -> Option<String> {
        if let Some(d) = self.display_name.as_ref().filter(|s| !s.trim().is_empty()) {
            return Some(d.clone());
        }
        if let Some(name) = &self.name {
            if let Some(f) = name.formatted.as_ref().filter(|s| !s.trim().is_empty()) {
                return Some(f.clone());
            }
            let parts: Vec<&str> = [name.given_name.as_deref(), name.family_name.as_deref()]
                .into_iter()
                .flatten()
                .filter(|s| !s.trim().is_empty())
                .collect();
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
        }
        None
    }

    fn primary_email(&self) -> Option<String> {
        self.emails
            .iter()
            .find(|e| e.primary)
            .or_else(|| self.emails.first())
            .and_then(|e| e.value.clone())
            .filter(|s| !s.trim().is_empty())
    }
}

/// Render a member + its SCIM link as a SCIM User resource (RFC 7643).
fn user_resource(member: &Member, scim: &ScimUser, email: Option<String>) -> Value {
    let mut resource = json!({
        "schemas": [USER_SCHEMA],
        "id": member.id.0.to_string(),
        "userName": member.handle,
        "active": scim.active,
        "meta": {
            "resourceType": "User",
            "created": scim.created_at.to_rfc3339(),
            "lastModified": scim.updated_at.to_rfc3339(),
            "location": format!("/scim/v2/Users/{}", member.id.0),
        }
    });
    if let Some(ext) = &scim.external_id {
        resource["externalId"] = json!(ext);
    }
    if let Some(dn) = &member.display_name {
        resource["displayName"] = json!(dn);
        resource["name"] = json!({ "formatted": dn });
    }
    if let Some(addr) = email {
        resource["emails"] = json!([{ "value": addr, "primary": true }]);
    }
    resource
}

/// Load the member + SCIM link + email for a resource, scoped to the workspace.
async fn load_resource(
    state: &AppState,
    workspace_id: WorkspaceId,
    member_id: maidan_types::MemberId,
) -> Option<Value> {
    let member = state.store.get_member(member_id).await.ok()?;
    if member.workspace_id != workspace_id {
        return None;
    }
    let scim = state.store.get_scim_user(member_id).await.ok()??;
    let email = state
        .store
        .get_member_email(member_id)
        .await
        .ok()
        .flatten()
        .map(|e| e.email);
    Some(user_resource(&member, &scim, email))
}

/// Revoke every API token of a member — the real effect of SCIM deactivation /
/// deprovisioning (best-effort; a failed revoke is logged, not fatal).
async fn revoke_member_tokens(
    state: &AppState,
    workspace_id: WorkspaceId,
    member_id: maidan_types::MemberId,
) {
    if let Ok(tokens) = state
        .store
        .list_api_tokens_for_member(workspace_id, member_id)
        .await
    {
        for token in tokens {
            if token.revoked_at.is_none() {
                if let Err(err) = state.store.revoke_api_token(token.id).await {
                    tracing::warn!(error = %err, "scim: revoking member token failed");
                }
            }
        }
    }
}

/// `GET /scim/v2/ServiceProviderConfig` — advertise the supported features.
pub async fn service_provider_config(Extension(auth): Extension<AuthContext>) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    scim_response(
        StatusCode::OK,
        json!({
            "schemas": [SPC_SCHEMA],
            "patch": { "supported": true },
            "bulk": { "supported": false, "maxOperations": 0, "maxPayloadSize": 0 },
            "filter": { "supported": true, "maxResults": 200 },
            "changePassword": { "supported": false },
            "sort": { "supported": false },
            "etag": { "supported": false },
            "authenticationSchemes": [{
                "type": "oauthbearertoken",
                "name": "OAuth Bearer Token",
                "description": "Authentication with a Maidan token:admin bearer token"
            }]
        }),
    )
}

/// `POST /scim/v2/Users` — provision a new member from a SCIM User.
pub async fn create_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    body: String,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let input: ScimUserInput = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return scim_error(StatusCode::BAD_REQUEST, &format!("invalid SCIM User: {e}")),
    };
    let user_name = match input
        .user_name
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(u) => u.to_string(),
        None => return scim_error(StatusCode::BAD_REQUEST, "userName is required"),
    };
    // SCIM uniqueness (RFC 7644 §3.3): a duplicate userName is a 409.
    if state
        .store
        .get_member_by_handle(auth.workspace_id, &user_name)
        .await
        .is_ok()
    {
        return scim_error(
            StatusCode::CONFLICT,
            "a user with this userName already exists",
        );
    }
    let member = match state
        .store
        .create_member(NewMember {
            workspace_id: auth.workspace_id,
            handle: user_name,
            display_name: input.resolved_display_name(),
            kind: MemberKind::Human,
        })
        .await
    {
        Ok(m) => m,
        Err(err) => {
            return scim_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("create failed: {err}"),
            )
        }
    };
    let scim = match state
        .store
        .create_scim_user(
            member.id,
            auth.workspace_id,
            input.external_id.as_deref(),
            input.active,
        )
        .await
    {
        Ok(s) => s,
        Err(err) => {
            return scim_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("link failed: {err}"),
            )
        }
    };
    let email = input.primary_email();
    if let Some(addr) = &email {
        let _ = state.store.set_member_email(member.id, addr).await;
    }
    crate::audit::record(
        &state,
        maidan_types::NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "scim.user.create".into(),
            target_kind: Some("member".into()),
            target_id: Some(member.id.0),
            metadata: json!({ "userName": member.handle }),
        },
    )
    .await;
    scim_response(StatusCode::CREATED, user_resource(&member, &scim, email))
}

/// `GET /scim/v2/Users/:id`.
pub async fn get_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    match load_resource(&state, auth.workspace_id, maidan_types::MemberId(id)).await {
        Some(resource) => scim_response(StatusCode::OK, resource),
        None => scim_error(StatusCode::NOT_FOUND, "no such user"),
    }
}

/// `GET /scim/v2/Users` — list all SCIM users, or resolve `?filter=userName eq "x"`.
pub async fn list_users(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    RawQuery(query): RawQuery,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let filter = query.as_deref().and_then(parse_username_filter);
    let mut resources = Vec::new();
    if let Some(user_name) = filter {
        // The IdP's de-dup check: resolve exactly this userName.
        if let Ok(member) = state
            .store
            .get_member_by_handle(auth.workspace_id, &user_name)
            .await
        {
            if let Some(resource) = load_resource(&state, auth.workspace_id, member.id).await {
                resources.push(resource);
            }
        }
    } else {
        let links = state
            .store
            .list_scim_users(auth.workspace_id)
            .await
            .unwrap_or_default();
        for link in links {
            if let Some(resource) = load_resource(&state, auth.workspace_id, link.member_id).await {
                resources.push(resource);
            }
        }
    }
    let total = resources.len();
    scim_response(
        StatusCode::OK,
        json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": total,
            "startIndex": 1,
            "itemsPerPage": total,
            "Resources": resources,
        }),
    )
}

/// Parse a SCIM `userName eq "value"` filter from the raw query string. Returns
/// the target userName, or `None` for any other/absent filter (→ list all).
fn parse_username_filter(raw_query: &str) -> Option<String> {
    let filter = raw_query
        .split('&')
        .find_map(|kv| kv.strip_prefix("filter="))?;
    // Query strings encode spaces as `%20` or `+` (form-urlencoded); normalize the
    // literal `+` to a space before percent-decoding (a real `+` arrives as `%2B`).
    let form_normalized = filter.replace('+', " ");
    let decoded = urlencoding::decode(&form_normalized).ok()?;
    let trimmed = decoded.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower.strip_prefix("username eq ")?;
    // Take the same byte range from the original to preserve value case.
    let value = trimmed[trimmed.len() - rest.len()..].trim();
    let value = value.trim_matches('"');
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// `PUT /scim/v2/Users/:id` — replace: applies `active` + `externalId` (member
/// handle/displayName are immutable in this P3; a rename is a follow-up).
pub async fn replace_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    body: String,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let member_id = maidan_types::MemberId(id);
    if load_resource(&state, auth.workspace_id, member_id)
        .await
        .is_none()
    {
        return scim_error(StatusCode::NOT_FOUND, "no such user");
    }
    let input: ScimUserInput = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return scim_error(StatusCode::BAD_REQUEST, &format!("invalid SCIM User: {e}")),
    };
    apply_update(
        &state,
        auth.workspace_id,
        member_id,
        input.external_id.as_deref(),
        input.active,
    )
    .await
}

#[derive(Deserialize)]
struct ScimPatchOp {
    #[serde(rename = "Operations", default)]
    operations: Vec<ScimPatchOperation>,
}

#[derive(Deserialize)]
struct ScimPatchOperation {
    #[serde(default)]
    op: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    value: Value,
}

/// `PATCH /scim/v2/Users/:id` — the deactivation path: `replace` of `active`
/// (and `externalId`). Other paths are accepted-and-ignored (P3).
pub async fn patch_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    body: String,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let member_id = maidan_types::MemberId(id);
    let Some(current) = load_resource(&state, auth.workspace_id, member_id).await else {
        return scim_error(StatusCode::NOT_FOUND, "no such user");
    };
    let patch: ScimPatchOp = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return scim_error(StatusCode::BAD_REQUEST, &format!("invalid PatchOp: {e}")),
    };
    let _ = PATCH_SCHEMA; // documented schema for the request envelope
    let mut active = current["active"].as_bool().unwrap_or(true);
    let mut external_id = current["externalId"].as_str().map(|s| s.to_string());
    for op in &patch.operations {
        if !op.op.eq_ignore_ascii_case("replace") && !op.op.eq_ignore_ascii_case("add") {
            continue;
        }
        match op.path.as_deref() {
            Some(p) if p.eq_ignore_ascii_case("active") => {
                if let Some(v) = op.value.as_bool() {
                    active = v;
                }
            }
            Some(p) if p.eq_ignore_ascii_case("externalId") => {
                external_id = op.value.as_str().map(|s| s.to_string());
            }
            // A pathless replace carries an object of attributes.
            None => {
                if let Some(v) = op.value.get("active").and_then(|v| v.as_bool()) {
                    active = v;
                }
                if let Some(v) = op.value.get("externalId").and_then(|v| v.as_str()) {
                    external_id = Some(v.to_string());
                }
            }
            _ => {}
        }
    }
    apply_update(
        &state,
        auth.workspace_id,
        member_id,
        external_id.as_deref(),
        active,
    )
    .await
}

/// Apply an `active` + `externalId` update to a member's SCIM link, revoking the
/// member's tokens when deactivating.
async fn apply_update(
    state: &AppState,
    workspace_id: WorkspaceId,
    member_id: maidan_types::MemberId,
    external_id: Option<&str>,
    active: bool,
) -> Response {
    match state
        .store
        .update_scim_user(member_id, external_id, active)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return scim_error(StatusCode::NOT_FOUND, "no such user"),
        Err(err) => {
            return scim_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("update failed: {err}"),
            )
        }
    }
    if !active {
        revoke_member_tokens(state, workspace_id, member_id).await;
    }
    crate::audit::record(
        state,
        maidan_types::NewAuditEvent {
            actor_id: None,
            action: if active {
                "scim.user.update"
            } else {
                "scim.user.deactivate"
            }
            .into(),
            target_kind: Some("member".into()),
            target_id: Some(member_id.0),
            metadata: json!({ "active": active }),
        },
    )
    .await;
    match load_resource(state, workspace_id, member_id).await {
        Some(resource) => scim_response(StatusCode::OK, resource),
        None => scim_error(StatusCode::NOT_FOUND, "no such user"),
    }
}

/// `DELETE /scim/v2/Users/:id` — deprovision: revoke the member's tokens + remove
/// the SCIM link (the member row is preserved for message-authorship integrity).
pub async fn delete_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let member_id = maidan_types::MemberId(id);
    // Scope check + existence: the member must be a SCIM user in this workspace.
    match state.store.get_scim_user(member_id).await {
        Ok(Some(scim)) if scim.workspace_id == auth.workspace_id => {}
        Ok(_) => return scim_error(StatusCode::NOT_FOUND, "no such user"),
        Err(err) => {
            return scim_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("lookup failed: {err}"),
            )
        }
    }
    revoke_member_tokens(&state, auth.workspace_id, member_id).await;
    match state.store.delete_scim_user(member_id).await {
        Ok(_) => {
            crate::audit::record(
                &state,
                maidan_types::NewAuditEvent {
                    actor_id: Some(auth.member_id),
                    action: "scim.user.delete".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(member_id.0),
                    metadata: json!({}),
                },
            )
            .await;
            (StatusCode::NO_CONTENT, ()).into_response()
        }
        Err(err) => scim_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("delete failed: {err}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_username_eq_filter_case_preserving_value() {
        assert_eq!(
            parse_username_filter("filter=userName%20eq%20%22JDoe%22").as_deref(),
            Some("JDoe")
        );
        assert_eq!(
            parse_username_filter("startIndex=1&filter=userName+eq+%22a%40b.com%22").as_deref(),
            Some("a@b.com")
        );
        // A non-userName filter falls through to "list all".
        assert!(parse_username_filter("filter=active%20eq%20true").is_none());
        assert!(parse_username_filter("count=10").is_none());
    }
}
