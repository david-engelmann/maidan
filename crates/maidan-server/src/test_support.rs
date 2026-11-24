//! Real callers for tests: a workspace with an admin and a bearer token,
//! minted through the store the way `maidan init` mints the first one.
//!
//! The server has no unauthenticated mode, so a test that drives it over
//! HTTP sends a token like any client. Built only under `cfg(test)` or the
//! `test-support` feature, which only dev-dependencies enable.

use std::time::Duration;

use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_store::Store;
use maidan_types::{
    ApiTokenId, MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId,
};

/// A workspace, its first admin, and that admin's bearer secret.
#[derive(Debug, Clone)]
pub struct Tenant {
    pub workspace_id: WorkspaceId,
    pub admin_id: MemberId,
    /// The admin's bearer secret. It holds every capability.
    pub token: String,
}

impl Tenant {
    /// A client that sends the admin's bearer on every request.
    pub fn client(&self) -> reqwest::Client {
        bearer_client(&self.token)
    }

    /// A new member of this workspace and a token for it holding every
    /// capability.
    pub async fn member(&self, store: &dyn Store, handle: &str, kind: MemberKind) -> Caller {
        caller(store, self.workspace_id, handle, kind).await
    }
}

/// A member and its bearer secret.
#[derive(Debug, Clone)]
pub struct Caller {
    pub member_id: MemberId,
    pub token: String,
}

impl Caller {
    pub fn client(&self) -> reqwest::Client {
        bearer_client(&self.token)
    }
}

/// Opens `name` with a human admin `admin` holding every capability.
pub async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let (workspace, _) = store
        .create_workspace_with_event(NewWorkspace {
            name: name.to_string(),
        })
        .await
        .unwrap_or_else(|err| panic!("create workspace {name}: {err}"));
    let admin = caller(store, workspace.id, "admin", MemberKind::Human).await;
    Tenant {
        workspace_id: workspace.id,
        admin_id: admin.member_id,
        token: admin.token,
    }
}

/// A new member of `workspace_id` and a token for it holding every capability.
pub async fn caller(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    handle: &str,
    kind: MemberKind,
) -> Caller {
    let (member, _) = store
        .create_member_with_event(NewMember {
            workspace_id,
            handle: handle.to_string(),
            display_name: None,
            kind,
        })
        .await
        .unwrap_or_else(|err| panic!("create member {handle}: {err}"));
    let token = token(store, workspace_id, member.id, capability::all()).await;
    Caller {
        member_id: member.id,
        token,
    }
}

/// A bearer secret for an existing member, holding `capabilities`.
pub async fn token(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    capabilities: Vec<String>,
) -> String {
    mint(store, workspace_id, member_id, capabilities).await.1
}

/// A registration's creator: a new member of `workspace_id` holding every
/// capability, and the id of its token. A webhook, hook or slash command
/// seeded through the store records it as `created_by_token`, because a
/// registration acts only while the token that made it could make it again
/// (`registration_owner`), and one made without a token never acts.
pub async fn registrar(store: &dyn Store, workspace_id: WorkspaceId) -> ApiTokenId {
    let (member, _) = store
        .create_member_with_event(NewMember {
            workspace_id,
            handle: "registrar".to_string(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap_or_else(|err| panic!("create registrar: {err}"));
    mint(store, workspace_id, member.id, capability::all())
        .await
        .0
}

/// A token for an existing member, by id and bearer secret.
async fn mint(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    capabilities: Vec<String>,
) -> (ApiTokenId, String) {
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("test".into()),
            capabilities,
            expires_at: None,
        })
        .await
        .unwrap_or_else(|err| panic!("mint token: {err}"));
    (token.id, secret.as_str().to_string())
}

/// A bearer secret holding every capability for an existing member.
pub async fn admin_token(store: &dyn Store, member_id: MemberId) -> String {
    let member = store
        .get_member(member_id)
        .await
        .unwrap_or_else(|err| panic!("member {member_id:?}: {err}"));
    token(store, member.workspace_id, member_id, capability::all()).await
}

/// The `Authorization` value for `token`.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// A client that sends `token` as its bearer on every request.
pub fn bearer_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    let mut value = reqwest::header::HeaderValue::from_str(&bearer(token))
        .unwrap_or_else(|err| panic!("bearer header: {err}"));
    value.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, value);
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|err| panic!("client: {err}"))
}

/// A land-gate advisor whose providers are one stand-in on a loopback
/// address, speaking each provider's own protocol: it answers every yes/no
/// question it is asked as all clear (the work is done and tested, and no
/// warning sign shows) and accepts any key.
#[allow(clippy::disallowed_methods)] // a stand-in provider, not the API
pub async fn land_gate_advisor_stub() -> std::sync::Arc<crate::land_gate_advisor::LandGateAdvisor> {
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};

    async fn systemone(Json(body): Json<Value>) -> Json<Value> {
        let mut answers = serde_json::Map::new();
        if let Some(questions) = body["questions"].as_object() {
            for name in questions.keys() {
                let p = match name.as_str() {
                    "done" | "tests_passed" | "code_change" => 0.96,
                    _ => 0.03,
                };
                answers.insert(name.clone(), json!({ "type": "noul", "noul": p }));
            }
        }
        Json(json!({
            "model": "jev-stub",
            "answers": answers,
            "usage": { "input_tokens": 1, "output_tokens": 1 },
        }))
    }
    // `/v1/decisions` is OpenAI's path and Perplexity's. OpenAI's Decisions
    // API takes predicates in an array and answers them in order;
    // Perplexity's takes TypeSafe's request, a `state` and a map of nouls.
    async fn decisions(Json(body): Json<Value>) -> Json<Value> {
        if body.get("state").is_some() {
            let Json(mut answer) = systemone(Json(body)).await;
            answer["model"] = json!("pplx-stub");
            return Json(answer);
        }
        let answers: Vec<Value> = body["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|question| {
                let name = question["name"].as_str().unwrap_or_default();
                let p = match name {
                    "done" | "tests_passed" | "code_change" => 0.96,
                    _ => 0.03,
                };
                json!({ "type": "predicate", "name": name, "probability": p })
            })
            .collect();
        Json(json!({
            "model": "gpt-stub",
            "answers": answers,
            "usage": { "input_tokens": 1, "output_tokens": 0, "total_tokens": 1 },
        }))
    }
    // Google's Gemini Decisions API: nouls in a map, and no model named.
    async fn google(Json(body): Json<Value>) -> Json<Value> {
        let mut answers = serde_json::Map::new();
        if let Some(questions) = body["questions"].as_object() {
            for name in questions.keys() {
                let p: f64 = match name.as_str() {
                    "done" | "tests_passed" | "code_change" => 0.96,
                    _ => 0.03,
                };
                answers.insert(
                    name.clone(),
                    json!({ "type": "noul", "noul": p, "confidence": p.max(1.0 - p) }),
                );
            }
        }
        Json(json!({
            "answers": answers,
            "usage": { "total_tokens": 1, "total_input_tokens": 1, "total_output_tokens": 0 },
        }))
    }
    // Cloudflare's Workers AI: TypeSafe's request under the account and the
    // model in the path, the answer inside the API's envelope.
    async fn clef(Json(body): Json<Value>) -> Json<Value> {
        let Json(mut answer) = systemone(Json(body)).await;
        answer["model"] = json!("clef-stub");
        Json(json!({ "result": answer, "success": true, "errors": [], "messages": [] }))
    }
    async fn models() -> Json<Value> {
        Json(json!({ "data": [] }))
    }

    let app = Router::new()
        .route("/v1/systemone", post(systemone))
        .route("/v1/decisions", post(decisions))
        .route("/client/v4/accounts/{account}/ai/run/{*model}", post(clef))
        .route("/v1beta/decisions", post(google))
        .route("/v1/models", axum::routing::get(models))
        .route("/v1beta/models", axum::routing::get(models));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the stand-in provider");
    let addr = listener.local_addr().expect("stand-in provider address");
    maidan_store::attribution::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let base = format!("http://{addr}/")
        .parse()
        .expect("stand-in provider url");
    std::sync::Arc::new(
        crate::land_gate_advisor::LandGateAdvisor::with_http_keys(
            reqwest::Client::new(),
            base,
            [
                Some("stub-key".into()),
                Some("stub-key".into()),
                Some("stub-key".into()),
                Some("stub-key".into()),
                Some("0123456789abcdef0123456789abcdef:stub-key".into()),
            ],
            std::time::Duration::from_secs(5),
        )
        .expect("stand-in advisor"),
    )
}

/// A [`crate::connections::ConnectionFactory`] for tests: it hands out the
/// mock registered for a token instead of building a client that would call
/// a provider, and answers an identity check from the accounts registered.
/// A token with no account is refused, as a provider refuses a bad one.
#[derive(Default)]
pub struct MockConnections {
    github: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<dyn crate::github::GithubSender>>,
    >,
    slack: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<dyn crate::slack::SlackSender>>,
    >,
    linear: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<dyn crate::linear::LinearSender>>,
    >,
    accounts: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Every `(surface, token)` the provider was asked about.
    pub asked: std::sync::Mutex<Vec<(maidan_types::ConnectionSurface, String)>>,
}

impl MockConnections {
    /// The provider knows `token`, as `account`.
    pub fn account(&self, token: &str, account: &str) {
        lock(&self.accounts).insert(token.into(), account.into());
    }

    pub fn github_for(&self, token: &str, sender: std::sync::Arc<dyn crate::github::GithubSender>) {
        lock(&self.github).insert(token.into(), sender);
    }

    pub fn slack_for(&self, token: &str, sender: std::sync::Arc<dyn crate::slack::SlackSender>) {
        lock(&self.slack).insert(token.into(), sender);
    }

    pub fn linear_for(&self, token: &str, sender: std::sync::Arc<dyn crate::linear::LinearSender>) {
        lock(&self.linear).insert(token.into(), sender);
    }
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[async_trait::async_trait]
impl crate::connections::ConnectionFactory for MockConnections {
    fn github(&self, token: &str) -> std::sync::Arc<dyn crate::github::GithubSender> {
        lock(&self.github)
            .get(token)
            .cloned()
            .unwrap_or_else(|| panic!("no GitHub mock registered for this token"))
    }

    fn slack(&self, token: &str) -> std::sync::Arc<dyn crate::slack::SlackSender> {
        lock(&self.slack)
            .get(token)
            .cloned()
            .unwrap_or_else(|| panic!("no Slack mock registered for this token"))
    }

    fn linear(&self, token: &str) -> std::sync::Arc<dyn crate::linear::LinearSender> {
        lock(&self.linear)
            .get(token)
            .cloned()
            .unwrap_or_else(|| panic!("no Linear mock registered for this token"))
    }

    async fn verify(
        &self,
        surface: maidan_types::ConnectionSurface,
        token: &str,
    ) -> Result<String, crate::connections::VerifyError> {
        lock(&self.asked).push((surface, token.to_string()));
        lock(&self.accounts).get(token).cloned().ok_or_else(|| {
            crate::connections::VerifyError::Refused("Bad credentials (mock)".into())
        })
    }
}

/// Store a workspace's own connection the way `PUT .../connections/{surface}`
/// does, without the provider check: for a test that is about what a stored
/// connection changes. The state must hold an encryption key.
pub async fn connect(
    state: &crate::state::AppState,
    workspace_id: WorkspaceId,
    by: MemberId,
    surface: maidan_types::ConnectionSurface,
    token: &str,
    inbound_secret: Option<&str>,
) {
    let key = state
        .federation
        .encryption_key
        .as_deref()
        .unwrap_or_else(|| panic!("the state needs an encryption key to store a connection"));
    let connection = crate::connections::StoredConnection {
        token: token.into(),
        account: "test-account".into(),
        webhook_secret: inbound_secret.map(str::to_string),
        signing_secret: inbound_secret.map(str::to_string),
        connected_at: chrono::Utc::now(),
    };
    let value_ciphertext = maidan_auth::encrypt_peer_secret(&connection.to_plaintext(surface), key)
        .unwrap_or_else(|e| panic!("encrypt the connection: {e}"));
    state
        .store
        .create_secret(maidan_types::NewSecret {
            workspace_id,
            name: surface.secret_name().into(),
            value_ciphertext,
            created_by: by,
        })
        .await
        .unwrap_or_else(|e| panic!("store the connection: {e}"));
}
