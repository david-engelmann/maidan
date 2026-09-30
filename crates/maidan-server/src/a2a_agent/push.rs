//! Push notification configs (§3.1.7–3.1.10) and delivery (§4.3.3).
//!
//! A config's `token` and `authentication.credentials` are sealed with the
//! server's at-rest key (`FEDERATION_ENCRYPTION_KEY`) before they are stored
//! and are never returned; responses carry the url, id and auth scheme only.

use std::time::Duration;

use maidan_a2a::page_token::{decode_config_cursor, encode_config_cursor};
use maidan_a2a::{
    A2aError, A2aErrorKind, AuthenticationInfo, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, ListTaskPushNotificationConfigsRequest,
    ListTaskPushNotificationConfigsResponse, StreamResponse, Task, TaskPushNotificationConfig,
    NOTIFICATION_TOKEN_HEADER,
};
use maidan_auth::capability::WORKSPACE_WRITE;
use maidan_auth::{decrypt_peer_secret_rotating, encrypt_peer_secret, AuthContext};
use maidan_store::A2aPushConfigRow;
use uuid::Uuid;

use super::error::{denied, internal, store};
use super::ops::{find, page_size, Found};
use crate::state::AppState;

const MAX_ATTEMPTS: u32 = 3;
/// The most push configs one task may hold. Every task update is sent to each
/// of them, up to [`MAX_ATTEMPTS`] times, so an unbounded list made one update
/// into as many outbound requests as a caller cared to register.
/// Checked before the write, so adds racing each other can pass it together:
/// it bounds the fan-out, not an exact count.
pub(crate) const MAX_PUSH_CONFIGS_PER_TASK: usize = 10;

/// A validated config with its secrets sealed, not yet tied to a task.
pub(super) struct Prepared {
    config_id: String,
    url: String,
    token_ciphertext: Option<String>,
    auth_scheme: Option<String>,
    auth_credentials_ciphertext: Option<String>,
}

pub(super) fn prepare(
    state: &AppState,
    auth: &AuthContext,
    config: TaskPushNotificationConfig,
) -> Result<Prepared, A2aError> {
    auth.require_capability(WORKSPACE_WRITE).map_err(denied)?;
    let url = config.url.trim().to_string();
    if url.is_empty() {
        return Err(A2aError::invalid_params("url is required"));
    }
    let target = maidan_auth::validate_egress_target(&url)
        .map_err(|e| A2aError::invalid_params(e.to_string()))?;
    // A push carries the task and the caller's notification credentials, so it
    // goes over TLS. Plain http is only for a development receiver on this host,
    // behind the same flag that lets egress reach a private address.
    if !scheme_allowed(
        target.scheme(),
        maidan_auth::private_egress_explicitly_allowed(),
    ) {
        return Err(A2aError::invalid_params(
            "push notification url must be https",
        ));
    }
    let seal = |secret: Option<String>| -> Result<Option<String>, A2aError> {
        let Some(secret) = secret.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let key = state.webhooks.encryption_key.as_deref().ok_or_else(|| {
            internal("FEDERATION_ENCRYPTION_KEY must be set to store push notification credentials")
        })?;
        encrypt_peer_secret(&secret, key)
            .map(Some)
            .map_err(internal)
    };
    let (auth_scheme, credentials) = match config.authentication {
        Some(info) if !info.scheme.trim().is_empty() => {
            (Some(info.scheme.trim().to_string()), info.credentials)
        }
        Some(_) => {
            return Err(A2aError::invalid_params(
                "authentication.scheme is required",
            ))
        }
        None => (None, None),
    };
    Ok(Prepared {
        config_id: config
            .id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| Uuid::now_v7().to_string()),
        url,
        token_ciphertext: seal(config.token)?,
        auth_scheme,
        auth_credentials_ciphertext: seal(credentials)?,
    })
}

fn scheme_allowed(scheme: &str, private_egress_allowed: bool) -> bool {
    scheme == "https" || private_egress_allowed
}

/// Store a prepared config for `task_id`.
pub(super) async fn attach(
    state: &AppState,
    task_id: &str,
    prepared: Prepared,
) -> Result<TaskPushNotificationConfig, A2aError> {
    let existing = state
        .store
        .list_a2a_task_push_configs(task_id)
        .await
        .map_err(store)?;
    let replaces = existing
        .iter()
        .any(|config| config.config_id == prepared.config_id);
    if !replaces && existing.len() >= MAX_PUSH_CONFIGS_PER_TASK {
        return Err(A2aError::invalid_params(format!(
            "a task holds at most {MAX_PUSH_CONFIGS_PER_TASK} push notification configs; \
             delete one before adding another"
        ))
        .with("taskId", task_id));
    }
    let row = A2aPushConfigRow {
        task_id: task_id.to_string(),
        config_id: prepared.config_id,
        url: prepared.url,
        token_ciphertext: prepared.token_ciphertext,
        auth_scheme: prepared.auth_scheme,
        auth_credentials_ciphertext: prepared.auth_credentials_ciphertext,
    };
    state
        .store
        .upsert_a2a_task_push_config(&row)
        .await
        .map_err(store)?;
    Ok(shown(row))
}

/// A stored config as responses show it: no secrets.
fn shown(row: A2aPushConfigRow) -> TaskPushNotificationConfig {
    TaskPushNotificationConfig {
        id: Some(row.config_id),
        task_id: row.task_id,
        url: row.url,
        token: None,
        authentication: row.auth_scheme.map(|scheme| AuthenticationInfo {
            scheme,
            credentials: None,
        }),
    }
}

/// The task a config request names, which must be a stored task the caller
/// can read.
async fn target(state: &AppState, auth: &AuthContext, task_id: &str) -> Result<(), A2aError> {
    auth.require_capability(WORKSPACE_WRITE).map_err(denied)?;
    if task_id.trim().is_empty() {
        return Err(A2aError::invalid_params("taskId is required"));
    }
    match find(state, auth, task_id).await? {
        Found::Task { .. } => Ok(()),
        Found::Gate(_) => Err(A2aError::new(
            A2aErrorKind::PushNotificationNotSupported,
            "approval gate tasks send no push notifications; subscribe to approval events instead",
        )
        .with("taskId", task_id)),
    }
}

pub(crate) async fn create(
    state: &AppState,
    auth: &AuthContext,
    config: TaskPushNotificationConfig,
) -> Result<TaskPushNotificationConfig, A2aError> {
    target(state, auth, &config.task_id).await?;
    let task_id = config.task_id.clone();
    let prepared = prepare(state, auth, config)?;
    attach(state, &task_id, prepared).await
}

pub(crate) async fn get(
    state: &AppState,
    auth: &AuthContext,
    req: GetTaskPushNotificationConfigRequest,
) -> Result<TaskPushNotificationConfig, A2aError> {
    target(state, auth, &req.task_id).await?;
    state
        .store
        .get_a2a_task_push_config(&req.task_id, &req.id)
        .await
        .map_err(store)?
        .map(shown)
        .ok_or_else(|| {
            A2aError::new(
                A2aErrorKind::TaskNotFound,
                "push notification config not found",
            )
            .with("taskId", &req.task_id)
            .with("configId", &req.id)
        })
}

/// A task's configs in id order, keyset-paged: `nextPageToken` encodes the
/// last config's id.
pub(crate) async fn list(
    state: &AppState,
    auth: &AuthContext,
    req: ListTaskPushNotificationConfigsRequest,
) -> Result<ListTaskPushNotificationConfigsResponse, A2aError> {
    target(state, auth, &req.task_id).await?;
    let page_size = page_size(req.page_size)?;
    let after = match req.page_token.as_deref().filter(|t| !t.is_empty()) {
        None => None,
        Some(token) => Some(decode_config_cursor(token)?),
    };
    let mut rows = state
        .store
        .page_a2a_task_push_configs(&req.task_id, after.as_deref(), i64::from(page_size) + 1)
        .await
        .map_err(store)?;
    let next_page_token = if rows.len() > page_size as usize {
        rows.truncate(page_size as usize);
        rows.last()
            .map(|row| encode_config_cursor(&row.config_id))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(ListTaskPushNotificationConfigsResponse {
        configs: rows.into_iter().map(shown).collect(),
        next_page_token,
    })
}

/// Deleting a config that is already gone succeeds: the call is idempotent.
pub(crate) async fn delete(
    state: &AppState,
    auth: &AuthContext,
    req: DeleteTaskPushNotificationConfigRequest,
) -> Result<(), A2aError> {
    target(state, auth, &req.task_id).await?;
    state
        .store
        .delete_a2a_task_push_config(&req.task_id, &req.id)
        .await
        .map_err(store)?;
    Ok(())
}

/// Notify every config of `task` of its current state, in the background.
pub(super) fn notify(state: &AppState, task: &Task) {
    let state = state.clone();
    let mut task = task.clone();
    task.history = None;
    let trace = maidan_store::trace::current();
    tokio::spawn(async move {
        let trace = trace;
        maidan_store::trace::maybe_scope(trace.clone(), async move {
        let configs = match state.store.list_a2a_task_push_configs(&task.id).await {
            Ok(configs) => configs,
            Err(err) => {
                tracing::error!(task_id = task.id, error = %err, "a2a push: listing configs failed");
                return;
            }
        };
        if configs.is_empty() {
            return;
        }
        let payload = match serde_json::to_value(StreamResponse::Task(task.clone())) {
            Ok(payload) => payload,
            Err(err) => {
                tracing::error!(task_id = task.id, error = %err, "a2a push: payload failed");
                return;
            }
        };
        for config in configs {
            let Some(headers) = PushHeaders::open(&state, &config) else {
                metrics::counter!("maidan_a2a_push_total", "result" => "failed").increment(1);
                continue;
            };
            let payload = payload.clone();
            let task_id = task.id.clone();
            let trace = trace.clone();
            tokio::spawn(async move {
                maidan_store::trace::maybe_scope(
                    trace,
                    deliver_a2a_push(&config.url, &payload, &task_id, &headers),
                )
                .await;
            });
        }
        })
        .await;
    });
}

/// The credentials a delivery presents, unsealed.
#[derive(Debug, Default)]
struct PushHeaders {
    authorization: Option<String>,
    token: Option<String>,
}

impl PushHeaders {
    fn open(state: &AppState, config: &A2aPushConfigRow) -> Option<Self> {
        let unseal = |sealed: &Option<String>| -> Result<Option<String>, ()> {
            let Some(sealed) = sealed else {
                return Ok(None);
            };
            let key = state.webhooks.encryption_key.as_deref().ok_or(())?;
            decrypt_peer_secret_rotating(sealed, key)
                .map(Some)
                .map_err(|_| ())
        };
        let opened = unseal(&config.token_ciphertext).and_then(|token| {
            let credentials = unseal(&config.auth_credentials_ciphertext)?;
            Ok(Self {
                authorization: config.auth_scheme.as_ref().map(|scheme| match credentials {
                    Some(credentials) => format!("{scheme} {credentials}"),
                    None => scheme.clone(),
                }),
                token,
            })
        });
        match opened {
            Ok(headers) => Some(headers),
            Err(()) => {
                tracing::error!(
                    task_id = config.task_id,
                    config_id = config.config_id,
                    "a2a push: stored credentials could not be unsealed; not delivering"
                );
                None
            }
        }
    }
}

/// POST one notification with bounded retry and backoff. Best-effort (not a
/// durable outbox); every outcome is counted in `maidan_a2a_push_total`.
async fn deliver_a2a_push(
    url: &str,
    payload: &serde_json::Value,
    task_id: &str,
    headers: &PushHeaders,
) {
    let (client, target) = match crate::egress_http::client_for(url).await {
        Ok(target) => target,
        Err(err) => {
            metrics::counter!("maidan_a2a_push_total", "result" => "failed").increment(1);
            tracing::error!(task_id, error = %err, "a2a push target refused");
            return;
        }
    };
    let mut backoff = Duration::from_millis(200);
    for attempt in 1..=MAX_ATTEMPTS {
        let mut request = crate::trace_context::stamp(client.post(target.clone()))
            .json(payload)
            .timeout(Duration::from_secs(10));
        if let Some(authorization) = &headers.authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        if let Some(token) = &headers.token {
            request = request.header(NOTIFICATION_TOKEN_HEADER, token);
        }
        match request.send().await {
            Ok(resp) if resp.status().is_success() => {
                metrics::counter!("maidan_a2a_push_total", "result" => "ok").increment(1);
                return;
            }
            Ok(resp) => {
                tracing::warn!(task_id, attempt, status = %resp.status(), "a2a push got non-success status");
            }
            Err(err) => {
                tracing::warn!(task_id, attempt, error = %err, "a2a push request failed");
            }
        }
        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
    }
    metrics::counter!("maidan_a2a_push_total", "result" => "failed").increment(1);
    tracing::error!(
        task_id,
        attempts = MAX_ATTEMPTS,
        "a2a push gave up after retries"
    );
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // mock servers, not the API
mod tests {

    #[test]
    fn a_push_url_is_https_unless_private_egress_is_on_for_development() {
        assert!(super::scheme_allowed("https", false));
        assert!(!super::scheme_allowed("http", false));
        assert!(super::scheme_allowed("http", true));
    }

    use super::*;
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    };

    use axum::{
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };

    type Seen = Arc<Mutex<Vec<(HeaderMap, serde_json::Value)>>>;

    /// A push endpoint that answers 500 to the first `fail_n` hits, then 200,
    /// recording each request's headers and body.
    async fn push_server(fail_n: u32) -> (String, Arc<AtomicU32>, Seen) {
        let hits = Arc::new(AtomicU32::new(0));
        let seen: Seen = Arc::default();
        let (hits_route, seen_route) = (hits.clone(), seen.clone());
        let app = Router::new().route(
            "/push",
            post(
                move |headers: HeaderMap, axum::Json(body): axum::Json<serde_json::Value>| {
                    let (hits, seen) = (hits_route.clone(), seen_route.clone());
                    async move {
                        seen.lock().unwrap().push((headers, body));
                        if hits.fetch_add(1, Ordering::SeqCst) < fail_n {
                            StatusCode::INTERNAL_SERVER_ERROR
                        } else {
                            StatusCode::OK
                        }
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/push"), hits, seen)
    }

    #[tokio::test]
    async fn a2a_push_retries_then_succeeds_with_credentials() {
        std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1");
        let (url, hits, seen) = push_server(2).await;
        let headers = PushHeaders {
            authorization: Some("Bearer s3cret".into()),
            token: Some("tok".into()),
        };
        let payload = serde_json::json!({ "task": { "id": "t1" } });
        deliver_a2a_push(&url, &payload, "t1", &headers).await;
        assert_eq!(hits.load(Ordering::SeqCst), 3, "should retry up to success");
        let seen = seen.lock().unwrap();
        let (headers, body) = seen.last().unwrap();
        assert_eq!(headers["authorization"], "Bearer s3cret");
        assert_eq!(headers[NOTIFICATION_TOKEN_HEADER], "tok");
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(body, &payload);
    }

    #[tokio::test]
    async fn a2a_push_gives_up_after_max_attempts() {
        std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1");
        let (url, hits, seen) = push_server(u32::MAX).await;
        deliver_a2a_push(
            &url,
            &serde_json::json!({ "task": { "id": "t2" } }),
            "t2",
            &PushHeaders::default(),
        )
        .await;
        assert_eq!(hits.load(Ordering::SeqCst), MAX_ATTEMPTS);
        let seen = seen.lock().unwrap();
        assert!(seen[0].0.get("authorization").is_none());
        assert!(seen[0].0.get(NOTIFICATION_TOKEN_HEADER).is_none());
    }
}
