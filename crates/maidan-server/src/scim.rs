//! SCIM 2.0 user provisioning. A minimal RFC 7643/7644 endpoint so an IdP (Okta
//! / Entra ID / …) can provision and deprovision Maidan members:
//! `ServiceProviderConfig` + `/Users` create / read / list (with the `userName
//! eq` filter) / replace / patch / delete, and `/Groups` ([`crate::scim_groups`]).
//! A SCIM User maps to a member (`userName` = handle, `id` = member id); the
//! `maidan_scim_users` link tracks `externalId` + `active`. Changing `userName`
//! renames the member: the id stays, so everything attributed to it stays.
//! Deactivation (`active=false`) and delete revoke the member's API tokens.
//!
//! Scoped to the caller's workspace and gated on `token:admin`. These routes
//! are intentionally outside the OpenAPI doc + capability-map (like `/mcp`) —
//! SCIM has its own schema and error envelope; auth is enforced inline here.

use axum::extract::{FromRequest, Path, RawQuery, Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::{Member, MemberKind, NewMember, ScimUser, WorkspaceId};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
pub(crate) const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
pub(crate) const PATCH_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
const SPC_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";

/// A SCIM JSON response with the `application/scim+json` content type.
pub(crate) fn scim_response(status: StatusCode, value: Value) -> Response {
    let body = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
    (
        status,
        [(header::CONTENT_TYPE, "application/scim+json")],
        body,
    )
        .into_response()
}

/// A SCIM error envelope (RFC 7644 §3.12).
pub(crate) fn scim_error(status: StatusCode, detail: &str) -> Response {
    scim_response(
        status,
        json!({
            "schemas": [ERROR_SCHEMA],
            "status": status.as_u16().to_string(),
            "detail": detail,
        }),
    )
}

/// A SCIM error with its `scimType` (RFC 7644 §3.12, table 9), which an IdP
/// reads to tell a taken `userName` (`uniqueness`) from a bad request.
pub(crate) fn scim_error_typed(status: StatusCode, scim_type: &str, detail: &str) -> Response {
    scim_response(
        status,
        json!({
            "schemas": [ERROR_SCHEMA],
            "status": status.as_u16().to_string(),
            "scimType": scim_type,
            "detail": detail,
        }),
    )
}

/// An axum rejection in SCIM's error envelope, so a malformed id or an
/// oversized body is an error an IdP can read.
fn scim_rejected(status: StatusCode, detail: String) -> Response {
    let (status, detail) = crate::extract::rejection(status, detail);
    scim_error(status, &detail)
}

crate::extract::wrap_extractor!(
    /// A SCIM path parameter, rejected in SCIM's error envelope.
    ScimPath,
    parts Path,
    Response,
    scim_rejected
);

/// A SCIM request body as text (the handler parses it as SCIM JSON), rejected
/// in SCIM's error envelope when it is over the body-size limit or not UTF-8.
pub struct ScimText(pub String);

impl crate::routing::Checked for ScimText {}

impl<S> FromRequest<S> for ScimText
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Response> {
        String::from_request(req, state)
            .await
            .map(Self)
            .map_err(|e| scim_rejected(e.status(), e.body_text()))
    }
}

/// Require `token:admin` (bypass callers pass). Returns `Some(<403 error>)` when
/// the caller is not authorized, `None` when it may proceed.
pub(crate) fn require_admin(auth: &AuthContext) -> Option<Response> {
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

fn user_name_taken() -> Response {
    scim_error_typed(
        StatusCode::CONFLICT,
        "uniqueness",
        "a user with this userName already exists",
    )
}

/// A SCIM error not yet rendered, so a parser can return it in a `Result`
/// without carrying a whole `Response`.
#[derive(Debug)]
pub(crate) struct ScimFault {
    status: StatusCode,
    scim_type: &'static str,
    detail: String,
}

impl ScimFault {
    pub(crate) fn new(status: StatusCode, scim_type: &'static str, detail: &str) -> Self {
        Self {
            status,
            scim_type,
            detail: detail.to_string(),
        }
    }

    pub(crate) fn invalid_value(detail: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalidValue", detail)
    }
}

impl IntoResponse for ScimFault {
    fn into_response(self) -> Response {
        scim_error_typed(self.status, self.scim_type, &self.detail)
    }
}

/// A `userName` from a request: trimmed, and refused when empty.
fn user_name_from(value: &str) -> Result<String, ScimFault> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(ScimFault::invalid_value("userName must not be empty"))
    } else {
        Ok(trimmed.to_string())
    }
}

/// A SCIM boolean. Entra ID sends `"True"` and `"False"` as strings unless
/// the app opts into its RFC-compliant mode, so both spellings are accepted.
fn scim_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
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
    ScimText(body): ScimText,
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
        return user_name_taken();
    }
    // The member and its SCIM link are created together, with their record: a
    // failure leaves neither, so the provider's retry does not meet a member
    // with no link under the same userName.
    let actor = auth.actor_id;
    let (member, scim) = match state
        .store
        .scim_provision_audited(
            NewMember {
                workspace_id: auth.workspace_id,
                handle: user_name,
                display_name: input.resolved_display_name(),
                kind: MemberKind::Human,
            },
            input.external_id.as_deref(),
            input.active,
            Box::new(move |(member, _)| maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(member.workspace_id),
                actor_id: Some(actor),
                action: "scim.user.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(member.id.0),
                metadata: json!({ "userName": member.handle }),
            }),
        )
        .await
    {
        Ok(provisioned) => provisioned,
        // The lookup above is the usual refusal. The unique index is the one
        // that still holds if two creates race, including a case-only twin.
        Err(maidan_store::StoreError::Conflict(_)) => return user_name_taken(),
        Err(err) => {
            return scim_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("create failed: {err}"),
            )
        }
    };
    // A delivery address, not authority: a bad one does not undo the user.
    let email = input.primary_email();
    if let Some(addr) = &email {
        let _ = state.store.set_member_email(member.id, addr).await;
    }
    scim_response(StatusCode::CREATED, user_resource(&member, &scim, email))
}

/// `GET /scim/v2/Users/:id`.
pub async fn get_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
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
    let filter = match eq_filter(
        query.as_deref().unwrap_or_default(),
        &["username", "externalid", "id"],
        "Users supports userName, externalId and id with eq",
    ) {
        Ok(filter) => filter,
        Err(fault) => return fault.into_response(),
    };
    // An error must not read as "no such user": the IdP would provision a
    // duplicate.
    let links = match state.store.list_scim_users(auth.workspace_id).await {
        Ok(links) => links,
        Err(err) => {
            return scim_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("list failed: {err}"),
            )
        }
    };
    let matching: Vec<maidan_types::MemberId> = match &filter {
        None => links.iter().map(|link| link.member_id).collect(),
        // userName is not caseExact (RFC 7643 §4.1.1). The match is the
        // store's handle lookup, which folds case, not a scan of every member.
        Some((attr, value)) if attr == "username" => {
            match state
                .store
                .get_member_by_handle(auth.workspace_id, value)
                .await
            {
                Ok(member) if links.iter().any(|link| link.member_id == member.id) => {
                    vec![member.id]
                }
                Ok(_) | Err(maidan_store::StoreError::NotFound) => Vec::new(),
                Err(err) => {
                    return scim_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("list failed: {err}"),
                    )
                }
            }
        }
        Some((attr, value)) if attr == "externalid" => links
            .iter()
            .filter(|link| link.external_id.as_deref() == Some(value.as_str()))
            .map(|link| link.member_id)
            .collect(),
        Some((_, value)) => links
            .iter()
            .filter(|link| link.member_id.0.to_string().eq_ignore_ascii_case(value))
            .map(|link| link.member_id)
            .collect(),
    };
    let mut resources = Vec::new();
    for member_id in matching {
        if let Some(resource) = load_resource(&state, auth.workspace_id, member_id).await {
            resources.push(resource);
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

/// The `filter` query parameter as `attribute eq "value"` on one of
/// `supported` (lowercase attribute names); `None` when there is no filter.
/// Any other filter is `invalidFilter`. Answering it with the whole list, as
/// Users once did, lets an IdP that matches on it (say `externalId eq`) link
/// its record to the first user it gets back.
pub(crate) fn eq_filter(
    raw_query: &str,
    supported: &[&str],
    detail: &str,
) -> Result<Option<(String, String)>, ScimFault> {
    let Some(filter) = query_param(raw_query, "filter") else {
        return Ok(None);
    };
    match parse_eq_filter(&filter) {
        Some((attr, value)) if supported.contains(&attr.as_str()) => Ok(Some((attr, value))),
        _ => Err(ScimFault::new(
            StatusCode::BAD_REQUEST,
            "invalidFilter",
            detail,
        )),
    }
}

/// A decoded query parameter. Query strings encode spaces as `%20` or `+`
/// (form-urlencoded); the literal `+` becomes a space before percent-decoding
/// (a real `+` arrives as `%2B`).
pub(crate) fn query_param(raw_query: &str, name: &str) -> Option<String> {
    let raw = raw_query.split('&').find_map(|kv| {
        let (key, value) = kv.split_once('=')?;
        key.eq_ignore_ascii_case(name).then_some(value)
    })?;
    let form_normalized = raw.replace('+', " ");
    urlencoding::decode(&form_normalized)
        .ok()
        .map(|decoded| decoded.into_owned())
}

/// Parse `attribute eq "value"` (RFC 7644 §3.4.2.2) into the attribute,
/// lowercased, and the value, case kept. Anything else, including a compound
/// `and`/`or` filter, is `None`.
pub(crate) fn parse_eq_filter(filter: &str) -> Option<(String, String)> {
    let trimmed = filter.trim();
    let (attribute, rest) = trimmed.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let (operator, value) = rest.split_once(char::is_whitespace)?;
    if !operator.eq_ignore_ascii_case("eq") {
        return None;
    }
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    if value.is_empty() || value.contains('"') {
        return None;
    }
    Some((attribute.to_ascii_lowercase(), value.to_string()))
}

/// `PUT /scim/v2/Users/:id` — replace: applies `userName` (a rename), `active`
/// and `externalId`. Okta renames a user this way. `displayName` is not
/// changed after creation.
pub async fn replace_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
    ScimText(body): ScimText,
) -> Response {
    if let Some(resp) = require_admin(&auth) {
        return resp;
    }
    let member_id = maidan_types::MemberId(id);
    let Some(current) = load_resource(&state, auth.workspace_id, member_id).await else {
        return scim_error(StatusCode::NOT_FOUND, "no such user");
    };
    let input: ScimUserInput = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return scim_error(StatusCode::BAD_REQUEST, &format!("invalid SCIM User: {e}")),
    };
    let user_name = match input.user_name.as_deref().map(user_name_from) {
        Some(Ok(name)) => Some(name),
        Some(Err(fault)) => return fault.into_response(),
        None => None,
    };
    apply_update(
        &state,
        &auth,
        &current,
        member_id,
        UserUpdate {
            user_name,
            external_id: input.external_id,
            active: input.active,
        },
    )
    .await
}

#[derive(Deserialize)]
pub(crate) struct ScimPatchOp {
    #[serde(rename = "Operations", default)]
    pub(crate) operations: Vec<ScimPatchOperation>,
}

#[derive(Deserialize)]
pub(crate) struct ScimPatchOperation {
    #[serde(default)]
    pub(crate) op: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) value: Value,
}

/// `PATCH /scim/v2/Users/:id` — `replace`/`add` of `userName` (a rename),
/// `active` and `externalId`, in either form an IdP sends: a `path` with a
/// value (Entra ID: `{"op":"Replace","path":"userName","value":"x"}`), or no
/// path and an object of attributes (Okta: `{"op":"replace","value":
/// {"active":false}}`). Other paths are accepted and ignored.
pub async fn patch_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
    ScimText(body): ScimText,
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
    let mut update = UserUpdate {
        user_name: None,
        active: current["active"].as_bool().unwrap_or(true),
        external_id: current["externalId"].as_str().map(|s| s.to_string()),
    };
    for op in &patch.operations {
        if !op.op.eq_ignore_ascii_case("replace") && !op.op.eq_ignore_ascii_case("add") {
            continue;
        }
        let pairs: Vec<(&str, &Value)> = match op.path.as_deref() {
            Some(path) => vec![(path, &op.value)],
            None => op
                .value
                .as_object()
                .map(|attrs| attrs.iter().map(|(k, v)| (k.as_str(), v)).collect())
                .unwrap_or_default(),
        };
        for (path, value) in pairs {
            if path.eq_ignore_ascii_case("active") {
                if let Some(v) = scim_bool(value) {
                    update.active = v;
                }
            } else if path.eq_ignore_ascii_case("externalId") {
                update.external_id = value.as_str().map(|s| s.to_string());
            } else if path.eq_ignore_ascii_case("userName") {
                let Some(name) = value.as_str() else {
                    return scim_error_typed(
                        StatusCode::BAD_REQUEST,
                        "invalidValue",
                        "userName must be a string",
                    );
                };
                match user_name_from(name) {
                    Ok(name) => update.user_name = Some(name),
                    Err(fault) => return fault.into_response(),
                }
            }
        }
    }
    apply_update(&state, &auth, &current, member_id, update).await
}

/// What a PUT or PATCH sets on a SCIM user.
struct UserUpdate {
    /// `None` keeps the current `userName`.
    user_name: Option<String>,
    external_id: Option<String>,
    active: bool,
}

/// Apply a user update: a rename, `active` and `externalId`, revoking the
/// member's tokens when deactivating. `current` is the resource before it.
async fn apply_update(
    state: &AppState,
    auth: &AuthContext,
    current: &Value,
    member_id: maidan_types::MemberId,
    update: UserUpdate,
) -> Response {
    let previous = current["userName"].as_str().unwrap_or_default();
    let rename = update.user_name.filter(|name| name != previous);
    let mut metadata = json!({ "active": update.active });
    if let Some(name) = &rename {
        metadata["userName"] = json!({ "from": previous, "to": name });
    }
    // The rename, the link and any token revocation commit together, with
    // their record. If any of it fails the provider sees an error and
    // retries, instead of a 200 with tokens still live.
    let event = maidan_types::NewAuditEvent {
        scope: maidan_types::AuditScope::Workspace(auth.workspace_id),
        actor_id: Some(auth.actor_id),
        action: if update.active {
            "scim.user.update"
        } else {
            "scim.user.deactivate"
        }
        .into(),
        target_kind: Some("member".into()),
        target_id: Some(member_id.0),
        metadata,
    };
    match state
        .store
        .scim_update_user_audited(
            auth.workspace_id,
            member_id,
            rename.as_deref(),
            update.external_id.as_deref(),
            update.active,
            event,
        )
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return scim_error(StatusCode::NOT_FOUND, "no such user"),
        Err(maidan_store::StoreError::Conflict(_)) => return user_name_taken(),
        Err(err) => {
            return scim_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("update failed: {err}"),
            )
        }
    }
    match load_resource(state, auth.workspace_id, member_id).await {
        Some(resource) => scim_response(StatusCode::OK, resource),
        None => scim_error(StatusCode::NOT_FOUND, "no such user"),
    }
}

/// `DELETE /scim/v2/Users/:id` — deprovision: revoke the member's tokens + remove
/// the SCIM link (the member row is preserved for message-authorship integrity).
pub async fn delete_user(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ScimPath(id): ScimPath<uuid::Uuid>,
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
    let event = maidan_types::NewAuditEvent {
        scope: maidan_types::AuditScope::Workspace(auth.workspace_id),
        actor_id: Some(auth.actor_id),
        action: "scim.user.delete".into(),
        target_kind: Some("member".into()),
        target_id: Some(member_id.0),
        metadata: json!({}),
    };
    match state
        .store
        .scim_deprovision_audited(auth.workspace_id, member_id, event)
        .await
    {
        Ok(_) => (StatusCode::NO_CONTENT, ()).into_response(),
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
    fn parses_eq_filters_and_refuses_the_rest() {
        let users = ["username", "externalid", "id"];
        let parse = |q: &str| eq_filter(q, &users, "x").map_err(|f| f.scim_type);
        assert_eq!(
            parse("filter=userName%20eq%20%22JDoe%22"),
            Ok(Some(("username".into(), "JDoe".into())))
        );
        assert_eq!(
            parse("startIndex=1&filter=userName+eq+%22a%40b.com%22"),
            Ok(Some(("username".into(), "a@b.com".into())))
        );
        assert_eq!(
            parse("filter=externalId%20eq%20%22okta-1%22"),
            Ok(Some(("externalid".into(), "okta-1".into())))
        );
        assert_eq!(parse("count=10"), Ok(None));
        for refused in [
            "filter=active%20eq%20true",
            "filter=userName%20sw%20%22j%22",
            "filter=userName%20eq%20%22a%22%20or%20userName%20eq%20%22b%22",
            "filter=",
        ] {
            assert_eq!(parse(refused), Err("invalidFilter"), "{refused}");
        }
    }
}
