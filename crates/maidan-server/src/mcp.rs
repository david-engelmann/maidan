//! HTTP transport for the MCP server: `POST /mcp` accepts a single JSON-RPC 2.0
//! request **or a batch** (a top-level array) and returns the corresponding
//! response(s). `POST /mcp/worker` and `POST /mcp/reviewer` are the same
//! transport with a fixed tool profile. Notifications (requests without an
//! `id`) are executed for effect and answered with `202 Accepted` and no body.
//! The `MCP-Protocol-Version` header is validated against the supported set.
//! Resource subscription notifications also stream on `GET /mcp/notifications`.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use maidan_auth::AuthContext;
use maidan_mcp::protocol::RequestBody;
use maidan_mcp::{is_supported_protocol_version, JsonRpcError, JsonRpcRequest, JsonRpcResponse};

use crate::error::ApiError;
use crate::extract::ApiBytes;
use crate::state::AppState;

/// Validate the `MCP-Protocol-Version` header (MCP spec: clients send it on
/// requests after `initialize`). Absent is allowed for backwards compatibility;
/// present-but-unsupported is a `400`.
pub(crate) fn validate_protocol_version(headers: &HeaderMap) -> Result<(), ApiError> {
    if let Some(value) = headers.get("mcp-protocol-version") {
        let version = value
            .to_str()
            .map_err(|_| ApiError::BadRequest("invalid MCP-Protocol-Version header".into()))?;
        if !is_supported_protocol_version(version) {
            return Err(ApiError::BadRequest(format!(
                "unsupported MCP-Protocol-Version: {version}"
            )));
        }
    }
    Ok(())
}

/// Whether a streamable request asks for the `2024-11-05` SSE-session model —
/// the only revision that has one. Every later revision is stateless, so the
/// session model is opt-in rather than the default:
///
/// - a request naming a version in `MCP-Protocol-Version` gets that version's
///   model (the header is already validated by [`validate_protocol_version`]);
/// - an `initialize` gets the model of the version it negotiates, because the
///   header only exists once a version has been agreed;
/// - anything else is stateless. A `2025-03-26` client never sends the header,
///   so defaulting to sessions would hand every such client a session it has no
///   reason to expect.
///
/// A follow-up on an already-open session is routed by its `Mcp-Session-Id`
/// before this is asked.
/// Which MCP revision a request speaks. A `2026-07-28` request states its
/// revision in `MCP-Protocol-Version` and in `params._meta`; a request on an
/// earlier revision states one of those older revisions in the header, or no
/// revision at all. Each era keeps its own rules, so a client of either is
/// served by the same endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Era {
    Legacy,
    Current,
}

const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// The revisions before `2026-07-28`, which keep `initialize`, `ping` and
/// `resources/subscribe`.
const LEGACY_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// Methods `2026-07-28` removed. On that revision each is not found (404).
const REMOVED_IN_2026: &[&str] = &[
    "initialize",
    "logging/setLevel",
    "ping",
    "resources/subscribe",
    "resources/unsubscribe",
];

const HEADER_MISMATCH: i32 = -32020;
const UNSUPPORTED_VERSION: i32 = -32022;
const INVALID_PARAMS: i32 = -32602;
const METHOD_NOT_FOUND: i32 = -32601;

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

fn meta_version(request: &JsonRpcRequest) -> Option<&serde_json::Value> {
    request.params.get("_meta")?.get(META_PROTOCOL_VERSION)
}

pub(crate) fn era(headers: &HeaderMap, request: &JsonRpcRequest) -> Era {
    match header(headers, "mcp-protocol-version") {
        Some(version) if LEGACY_VERSIONS.contains(&version) => Era::Legacy,
        Some(_) => Era::Current,
        None if meta_version(request).is_some() => Era::Current,
        None => Era::Legacy,
    }
}

/// A request refused before it reaches the server: a JSON-RPC error with the
/// request's id, and the HTTP status the revision prescribes.
fn refuse(
    status: StatusCode,
    request: &JsonRpcRequest,
    code: i32,
    message: String,
    data: Option<serde_json::Value>,
) -> Box<Response> {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);
    let error = JsonRpcError {
        code,
        message,
        data,
    };
    Box::new((status, Json(JsonRpcResponse::failure(id, error))).into_response())
}

/// The `2026-07-28` checks, in the order the revision gives them: the version
/// header, the request's own `_meta`, the two agreeing, the version being one
/// this server speaks, the routing headers, and a method the revision removed.
fn admit_current(headers: &HeaderMap, request: &JsonRpcRequest) -> Result<(), Box<Response>> {
    let bad = StatusCode::BAD_REQUEST;
    let Some(version) = header(headers, "mcp-protocol-version") else {
        return Err(refuse(
            bad,
            request,
            HEADER_MISMATCH,
            "the MCP-Protocol-Version header is required".into(),
            None,
        ));
    };
    let meta = request.params.get("_meta").and_then(|m| m.as_object());
    for key in [META_PROTOCOL_VERSION, META_CLIENT_CAPABILITIES] {
        if !meta.is_some_and(|m| m.contains_key(key)) {
            return Err(refuse(
                bad,
                request,
                INVALID_PARAMS,
                format!("params._meta must carry {key}"),
                None,
            ));
        }
    }
    let stated = meta_version(request)
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if stated != version {
        return Err(refuse(
            bad,
            request,
            HEADER_MISMATCH,
            format!("MCP-Protocol-Version is {version:?} but params._meta states {stated:?}"),
            None,
        ));
    }
    if !is_supported_protocol_version(version) {
        return Err(refuse(
            bad,
            request,
            UNSUPPORTED_VERSION,
            format!("unsupported protocol version {version:?}"),
            Some(serde_json::json!({
                "requested": version,
                "supported": maidan_mcp::SUPPORTED_PROTOCOL_VERSIONS,
            })),
        ));
    }
    match header(headers, "mcp-method") {
        Some(method) if method == request.method => {}
        Some(method) => {
            return Err(refuse(
                bad,
                request,
                HEADER_MISMATCH,
                format!(
                    "Mcp-Method {method:?} does not match the request's method {:?}",
                    request.method
                ),
                None,
            ))
        }
        None => {
            return Err(refuse(
                bad,
                request,
                HEADER_MISMATCH,
                "the Mcp-Method header is required".into(),
                None,
            ))
        }
    }
    if let Some(target) = request_routing_name(request) {
        match header(headers, "mcp-name") {
            Some(name) if name == target => {}
            Some(name) => {
                return Err(refuse(
                    bad,
                    request,
                    HEADER_MISMATCH,
                    format!("Mcp-Name {name:?} does not match the request's target {target:?}"),
                    None,
                ))
            }
            None => {
                return Err(refuse(
                    bad,
                    request,
                    HEADER_MISMATCH,
                    format!("the Mcp-Name header is required for {}", request.method),
                    None,
                ))
            }
        }
    }
    if REMOVED_IN_2026.contains(&request.method.as_str()) {
        return Err(refuse(
            StatusCode::NOT_FOUND,
            request,
            METHOD_NOT_FOUND,
            format!(
                "method not found: {} is not part of protocol version 2026-07-28",
                request.method
            ),
            None,
        ));
    }
    Ok(())
}

/// Admit a parsed request: on `2026-07-28` every check that revision makes,
/// otherwise the earlier revisions' rule that routing headers, when sent, match.
pub(crate) fn admit(headers: &HeaderMap, request: &JsonRpcRequest) -> Result<Era, Box<Response>> {
    match era(headers, request) {
        Era::Current => admit_current(headers, request).map(|()| Era::Current),
        Era::Legacy => validate_routing_headers(headers, request)
            .map(|()| Era::Legacy)
            .map_err(|err| Box::new(err.into_response())),
    }
}

/// The HTTP response for a dispatched request. `2026-07-28` answers a method
/// the server does not have with 404 as well as -32601.
pub(crate) fn reply(era: Era, response: JsonRpcResponse) -> Response {
    let not_found = era == Era::Current
        && response
            .error
            .as_ref()
            .is_some_and(|e| e.code == METHOD_NOT_FOUND);
    if not_found {
        (StatusCode::NOT_FOUND, Json(response)).into_response()
    } else {
        Json(response).into_response()
    }
}

pub(crate) fn wants_session(headers: &HeaderMap, request: &JsonRpcRequest) -> bool {
    if let Some(version) = headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok())
    {
        return version == maidan_mcp::SESSION_PROTOCOL_VERSION;
    }
    request.method == "initialize"
        && maidan_mcp::negotiate_protocol_version(
            request
                .params
                .get("protocolVersion")
                .and_then(|v| v.as_str()),
        ) == maidan_mcp::SESSION_PROTOCOL_VERSION
}

/// The name a routing gateway would put in `Mcp-Name` for a request: the tool /
/// prompt name, or the resource uri. `None` for methods that name no target
/// (`tools/list`, `initialize`, …).
fn request_routing_name(request: &JsonRpcRequest) -> Option<&str> {
    match request.method.as_str() {
        "tools/call" | "prompts/get" => request.params.get("name").and_then(|v| v.as_str()),
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => {
            request.params.get("uri").and_then(|v| v.as_str())
        }
        _ => None,
    }
}

/// Validate the SEP-2243 routing headers (`Mcp-Method` / `Mcp-Name`) against the
/// request body. Both are optional (a gateway adds them so it can route/authorize
/// without parsing JSON; a direct client may omit them). When present they MUST
/// match the body — a gateway that routed on a header must not be handed a
/// contradicting body — so a mismatch is a `400`. An `Mcp-Name` on a method that
/// names no target is ignored (the body does no more than the header authorized).
pub(crate) fn validate_routing_headers(
    headers: &HeaderMap,
    request: &JsonRpcRequest,
) -> Result<(), ApiError> {
    if let Some(m) = headers.get("mcp-method").and_then(|v| v.to_str().ok()) {
        if m != request.method {
            return Err(ApiError::BadRequest(format!(
                "Mcp-Method header {:?} does not match request method {:?}",
                m, request.method
            )));
        }
    }
    if let Some(n) = headers.get("mcp-name").and_then(|v| v.to_str().ok()) {
        if let Some(name) = request_routing_name(request) {
            if name != n {
                return Err(ApiError::BadRequest(format!(
                    "Mcp-Name header {n:?} does not match request target {name:?}"
                )));
            }
        }
    }
    Ok(())
}

pub async fn handler(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    ApiBytes(body): ApiBytes,
) -> Response {
    json_rpc(state, auth, headers, body, None).await
}

/// `POST /mcp/worker`: the worker profile's fixed tool list.
pub async fn worker(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    ApiBytes(body): ApiBytes,
) -> Response {
    json_rpc(
        state,
        auth,
        headers,
        body,
        Some(maidan_mcp::Profile::Worker),
    )
    .await
}

/// `POST /mcp/reviewer`: the reviewer profile's fixed tool list.
pub async fn reviewer(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    ApiBytes(body): ApiBytes,
) -> Response {
    json_rpc(
        state,
        auth,
        headers,
        body,
        Some(maidan_mcp::Profile::Reviewer),
    )
    .await
}

async fn json_rpc(
    state: AppState,
    auth: AuthContext,
    headers: HeaderMap,
    body: axum::body::Bytes,
    profile: Option<maidan_mcp::Profile>,
) -> Response {
    match maidan_mcp::protocol::parse_body(&body) {
        Err(rejected) => Json(JsonRpcResponse::rejected(rejected)).into_response(),
        // Routing headers describe a single op; a batch names many, so they are
        // validated per single request, not against an array. Batches belong
        // to the earlier revisions, so a batch keeps their header check.
        Ok(RequestBody::Batch(items)) => {
            if let Err(err) = validate_protocol_version(&headers) {
                return err.into_response();
            }
            batch_response(&state, &auth, items, profile).await
        }
        Ok(RequestBody::Single(request)) => {
            single_response(&state, &auth, &headers, request, profile).await
        }
    }
}

async fn single_response(
    state: &AppState,
    auth: &AuthContext,
    headers: &HeaderMap,
    request: JsonRpcRequest,
    profile: Option<maidan_mcp::Profile>,
) -> Response {
    let era = match admit(headers, &request) {
        Ok(era) => era,
        Err(refused) => return *refused,
    };
    // A notification (no `id`) is executed for effect and returns no body.
    if request.id.is_none() {
        let _ = answer(state, auth, request, profile).await;
        return StatusCode::ACCEPTED.into_response();
    }
    if let Err(resp) = crate::mcp_quota::enforce_mcp_quota(state, auth, &request).await {
        return Json(resp).into_response();
    }
    reply(era, answer(state, auth, request, profile).await)
}

async fn answer(
    state: &AppState,
    auth: &AuthContext,
    request: JsonRpcRequest,
    profile: Option<maidan_mcp::Profile>,
) -> JsonRpcResponse {
    match profile {
        Some(profile) => state.mcp.handle_profile(request, auth, profile).await,
        None => state.mcp.handle(request, auth).await,
    }
}

async fn batch_response(
    state: &AppState,
    auth: &AuthContext,
    items: Vec<Result<JsonRpcRequest, JsonRpcError>>,
    profile: Option<maidan_mcp::Profile>,
) -> Response {
    let mut responses = Vec::new();
    for item in items {
        let request = match item {
            Ok(request) => request,
            Err(rejected) => {
                responses.push(JsonRpcResponse::rejected(rejected));
                continue;
            }
        };
        let is_notification = request.id.is_none();
        if let Err(resp) = crate::mcp_quota::enforce_mcp_quota(state, auth, &request).await {
            if !is_notification {
                responses.push(resp);
            }
            continue;
        }
        let resp = answer(state, auth, request, profile).await;
        if !is_notification {
            responses.push(resp);
        }
    }
    // A batch of only notifications yields no response body.
    if responses.is_empty() {
        StatusCode::ACCEPTED.into_response()
    } else {
        Json(responses).into_response()
    }
}

#[cfg(test)]
mod era_tests {
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use serde_json::{json, Value};

    use super::*;

    fn request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(7)),
            method: method.into(),
            params,
        }
    }

    fn meta(version: &str) -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": version,
            "io.modelcontextprotocol/clientCapabilities": {}
        })
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    /// The status and JSON-RPC error a refusal carries, or `None` if admitted.
    fn refused(headers: &HeaderMap, request: &JsonRpcRequest) -> Option<(StatusCode, i64, Value)> {
        let response = *admit(headers, request).err()?;
        let status = response.status();
        let body = futures::executor::block_on(axum::body::to_bytes(response.into_body(), 1 << 16))
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["id"],
            json!(7),
            "a refusal echoes the request id: {body}"
        );
        Some((
            status,
            body["error"]["code"].as_i64().unwrap(),
            body["error"]["data"].clone(),
        ))
    }

    fn current(method: &str) -> [(&'static str, String); 2] {
        [
            ("mcp-protocol-version", "2026-07-28".into()),
            ("mcp-method", method.into()),
        ]
    }

    fn with(pairs: &[(&'static str, String)]) -> HeaderMap {
        headers(
            &pairs
                .iter()
                .map(|(k, v)| (*k, v.as_str()))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn the_era_follows_the_header_then_the_meta() {
        let bare = request("tools/list", json!({}));
        assert_eq!(era(&HeaderMap::new(), &bare), Era::Legacy);
        assert_eq!(
            era(&headers(&[("mcp-protocol-version", "2025-11-25")]), &bare),
            Era::Legacy
        );
        assert_eq!(
            era(&headers(&[("mcp-protocol-version", "2026-07-28")]), &bare),
            Era::Current
        );
        assert_eq!(
            era(&headers(&[("mcp-protocol-version", "v999")]), &bare),
            Era::Current
        );
        let stated = request("tools/list", json!({ "_meta": meta("2026-07-28") }));
        assert_eq!(era(&HeaderMap::new(), &stated), Era::Current);
    }

    #[test]
    fn a_complete_current_request_is_admitted() {
        let call = request(
            "tools/call",
            json!({ "_meta": meta("2026-07-28"), "name": "whoami" }),
        );
        let mut pairs = current("tools/call").to_vec();
        pairs.push(("mcp-name", "  whoami  ".into()));
        assert!(refused(&with(&pairs), &call).is_none());
    }

    #[test]
    fn each_current_rule_refuses_with_the_revisions_code_and_status() {
        let h = with(&current("tools/list"));
        // _meta missing, or missing a required key: -32602.
        assert_eq!(
            refused(&h, &request("tools/list", json!({}))).unwrap().1,
            -32602
        );
        let no_caps =
            json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } });
        assert_eq!(
            refused(&h, &request("tools/list", no_caps)).unwrap().1,
            -32602
        );
        // Header and _meta disagree: -32020, before the version is judged.
        let (status, code, _) =
            refused(&h, &request("tools/list", json!({ "_meta": meta("v999") }))).unwrap();
        assert_eq!((status, code), (StatusCode::BAD_REQUEST, -32020));
        // Unsupported version: -32022 with requested and supported.
        let v999 = headers(&[
            ("mcp-protocol-version", "v999"),
            ("mcp-method", "tools/list"),
        ]);
        let (status, code, data) = refused(
            &v999,
            &request("tools/list", json!({ "_meta": meta("v999") })),
        )
        .unwrap();
        assert_eq!((status, code), (StatusCode::BAD_REQUEST, -32022));
        assert_eq!(data["requested"], "v999");
        assert!(data["supported"]
            .as_array()
            .is_some_and(|s| s.contains(&json!("2026-07-28"))));
        // Routing headers required and matching: -32020.
        let ok_body = request("tools/list", json!({ "_meta": meta("2026-07-28") }));
        let no_method = headers(&[("mcp-protocol-version", "2026-07-28")]);
        assert_eq!(refused(&no_method, &ok_body).unwrap().1, -32020);
        let wrong_case = headers(&[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "TOOLS/LIST"),
        ]);
        assert_eq!(refused(&wrong_case, &ok_body).unwrap().1, -32020);
        let call = request(
            "tools/call",
            json!({ "_meta": meta("2026-07-28"), "name": "whoami" }),
        );
        assert_eq!(
            refused(&with(&current("tools/call")), &call).unwrap().1,
            -32020,
            "Mcp-Name missing"
        );
        let mut wrong_name = current("tools/call").to_vec();
        wrong_name.push(("mcp-name", "other".into()));
        assert_eq!(refused(&with(&wrong_name), &call).unwrap().1, -32020);
    }

    #[test]
    fn a_method_2026_removed_is_not_found_on_that_revision_only() {
        for method in REMOVED_IN_2026 {
            let removed = request(method, json!({ "_meta": meta("2026-07-28") }));
            let (status, code, _) = refused(&with(&current(method)), &removed).unwrap();
            assert_eq!((status, code), (StatusCode::NOT_FOUND, -32601), "{method}");
            let legacy = request(method, json!({}));
            assert!(
                refused(&headers(&[("mcp-protocol-version", "2025-11-25")]), &legacy).is_none(),
                "{method} stays on 2025-11-25"
            );
        }
    }

    #[test]
    fn a_legacy_request_keeps_optional_routing_headers() {
        let list = request("tools/list", json!({}));
        assert!(refused(&HeaderMap::new(), &list).is_none());
        assert!(refused(&headers(&[("mcp-protocol-version", "2025-06-18")]), &list).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    fn req(method: &str, params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn routing_headers_absent_are_ok() {
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&HeaderMap::new(), &r).is_ok());
    }

    #[test]
    fn matching_routing_headers_are_ok() {
        let mut h = HeaderMap::new();
        h.insert("mcp-method", HeaderValue::from_static("tools/call"));
        h.insert("mcp-name", HeaderValue::from_static("search"));
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&h, &r).is_ok());
    }

    #[test]
    fn mcp_method_mismatch_is_rejected() {
        let mut h = HeaderMap::new();
        h.insert("mcp-method", HeaderValue::from_static("tools/list"));
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&h, &r).is_err());
    }

    #[test]
    fn mcp_name_mismatch_is_rejected() {
        let mut h = HeaderMap::new();
        h.insert("mcp-name", HeaderValue::from_static("evil"));
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&h, &r).is_err());
    }

    #[test]
    fn mcp_name_on_a_method_that_names_no_target_is_ignored() {
        // A superfluous Mcp-Name on tools/list is safe (the body does no more than
        // the header authorized), so it's not a mismatch.
        let mut h = HeaderMap::new();
        h.insert("mcp-name", HeaderValue::from_static("whatever"));
        let r = req("tools/list", json!({}));
        assert!(validate_routing_headers(&h, &r).is_ok());
    }

    #[test]
    fn mcp_name_matches_resource_uri() {
        let mut h = HeaderMap::new();
        h.insert("mcp-name", HeaderValue::from_static("threads/42"));
        let r = req("resources/read", json!({ "uri": "threads/42" }));
        assert!(validate_routing_headers(&h, &r).is_ok());
    }
}
