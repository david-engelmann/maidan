//! Tenant isolation, as a conformance check over the whole HTTP surface.
//!
//! Workspace B holds a private channel, a thread, a message and a second
//! member. Workspace A's token carries every workspace-scoped capability.
//! Every operation in the served OpenAPI document whose path names one of B's
//! entities is called with A's token and B's ids, and each must refuse: no
//! 2xx, no B content in the body. B's entities must be intact afterwards.
//!
//! A new route is covered by being in the spec; nothing needs registering
//! here. A path parameter this file does not know gets a random id, which
//! proves nothing, so the known set is asserted to stay large.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use reqwest::Method;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

const SECRET_BODY: &str = "tenant-b-secret-body-7f3a";
const SECRET_CHANNEL: &str = "tenant-b-secret-channel";

struct Harness {
    addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        bearer: &str,
        body: Option<Value>,
    ) -> reqwest::Response {
        let mut req = self
            .client
            .request(method, self.url(path))
            .header("Authorization", format!("Bearer {bearer}"));
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send().await.unwrap_or_else(|e| panic!("{path}: {e}"))
    }
}

async fn spawn() -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    Harness {
        addr,
        _server: server,
        client,
        store,
        _dir: dir,
    }
}

async fn member(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    handle: &str,
    kind: MemberKind,
) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.to_string(),
            display_name: None,
            kind,
        })
        .await
        .unwrap()
        .id
}

async fn token(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    capabilities: Vec<String>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

/// Every capability that is scoped to the token's own workspace. The global
/// ones (`audit:read-global`, `operator:global`) and federation's peer
/// capabilities cross workspaces by design and are left out.
fn workspace_scoped_capabilities() -> Vec<String> {
    capability::all()
        .into_iter()
        .filter(|c| {
            ![
                capability::AUDIT_READ_GLOBAL,
                capability::OPERATOR_GLOBAL,
                capability::FEDERATION_INGEST,
                capability::FEDERATION_ADMIN,
            ]
            .contains(&c.as_str())
        })
        .collect()
}

/// The `id` of a created resource, or the response that refused it.
async fn created_id(res: reqwest::Response) -> String {
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    match body["id"].as_str() {
        Some(id) if status.is_success() => id.to_string(),
        _ => panic!("seeding tenant B failed: {status} {body}"),
    }
}

struct TenantB {
    workspace: String,
    channel: String,
    thread: String,
    message: String,
    member: String,
    bearer: String,
}

async fn seed_b(h: &Harness) -> TenantB {
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "tenant-b".into(),
        })
        .await
        .unwrap()
        .id;
    let owner = member(h.store.as_ref(), ws, "b-owner", MemberKind::Human).await;
    let other = member(h.store.as_ref(), ws, "b-other", MemberKind::Human).await;
    let bearer = token(h.store.as_ref(), ws, owner, capability::all()).await;

    let channel = h
        .send(
            Method::POST,
            &format!("/workspaces/{}/channels", ws.0),
            &bearer,
            Some(json!({ "name": SECRET_CHANNEL, "private": true })),
        )
        .await;
    let channel = created_id(channel).await;
    let thread = h
        .send(
            Method::POST,
            &format!("/channels/{channel}/threads"),
            &bearer,
            Some(json!({})),
        )
        .await;
    let thread = created_id(thread).await;
    let message = h
        .send(
            Method::POST,
            &format!("/threads/{thread}/messages"),
            &bearer,
            Some(json!({ "body": SECRET_BODY })),
        )
        .await;
    let message = created_id(message).await;

    TenantB {
        workspace: ws.0.to_string(),
        channel,
        thread,
        message,
        member: other.0.to_string(),
        bearer,
    }
}

/// B's id for a path parameter, keyed by the segment before it: `{id}` means
/// a thread under `/threads/` and a member under `/members/`.
fn b_id<'a>(b: &'a TenantB, segment: &str, param: &str) -> Option<&'a str> {
    Some(match (segment, param) {
        ("workspaces", "id" | "wid") => &b.workspace,
        ("channels", "id" | "cid") | ("channel-follows", "cid") => &b.channel,
        ("threads", "id" | "tid") | ("thread-follows", "tid") | ("dependencies", "dep_id") => {
            &b.thread
        }
        ("messages", "id" | "mid") => &b.message,
        ("members", "id" | "mid")
        | ("member-follows", "followed_id")
        | ("reviewers", "member_id") => &b.member,
        _ => return None,
    })
}

struct Probe {
    method: Method,
    path: String,
    template: String,
}

/// Each operation in the served spec that names at least one of B's
/// entities, with every parameter filled in.
fn probes(b: &TenantB) -> (Vec<Probe>, BTreeMap<String, usize>) {
    let doc = serde_json::to_value(maidan_server::openapi::document()).unwrap();
    let mut probes = Vec::new();
    let mut unknown = BTreeMap::<String, usize>::new();
    for (template, item) in doc["paths"].as_object().unwrap() {
        let segments: Vec<&str> = template.split('/').collect();
        let mut names_b = false;
        let mut filled = Vec::with_capacity(segments.len());
        for (i, seg) in segments.iter().enumerate() {
            let Some(param) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
                filled.push((*seg).to_string());
                continue;
            };
            let prev = if i > 0 { segments[i - 1] } else { "" };
            match b_id(b, prev, param) {
                Some(id) => {
                    names_b = true;
                    filled.push(id.to_string());
                }
                None => {
                    *unknown.entry(format!("{prev}/{{{param}}}")).or_default() += 1;
                    filled.push(Uuid::now_v7().to_string());
                }
            }
        }
        if !names_b {
            continue;
        }
        let path = filled.join("/");
        for method in item.as_object().unwrap().keys() {
            let Ok(method) = method.to_uppercase().parse::<Method>() else {
                continue;
            };
            if matches!(
                method,
                Method::GET | Method::POST | Method::PUT | Method::PATCH | Method::DELETE
            ) {
                probes.push(Probe {
                    method,
                    path: path.clone(),
                    template: template.clone(),
                });
            }
        }
    }
    (probes, unknown)
}

#[tokio::test]
async fn no_operation_serves_or_changes_another_workspace() {
    let h = spawn().await;
    let b = seed_b(&h).await;

    let ws_a = h
        .store
        .create_workspace(NewWorkspace {
            name: "tenant-a".into(),
        })
        .await
        .unwrap()
        .id;
    let a_member = member(h.store.as_ref(), ws_a, "a-agent", MemberKind::Agent).await;
    let a = token(
        h.store.as_ref(),
        ws_a,
        a_member,
        workspace_scoped_capabilities(),
    )
    .await;

    let (probes, unknown) = probes(&b);
    // The spec had 169 operations naming a workspace, channel, thread, message
    // or member when this was written. A drop means the mapping stopped
    // matching the spec, not that the surface shrank.
    assert!(
        probes.len() >= 150,
        "only {} operations probed",
        probes.len()
    );

    let mut leaks = Vec::new();
    let mut body_rejected = 0;
    for probe in &probes {
        let body =
            matches!(probe.method, Method::POST | Method::PUT | Method::PATCH).then(|| json!({}));
        let res = h.send(probe.method.clone(), &probe.path, &a, body).await;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let label = format!("{} {}", probe.method, probe.template);
        if status.is_success() || status.is_redirection() {
            leaks.push(format!("{label}: {status}"));
        } else if status.as_u16() == 429 {
            leaks.push(format!(
                "{label}: rate limited, so the check proved nothing"
            ));
        } else if text.contains(SECRET_BODY) || text.contains(SECRET_CHANNEL) {
            leaks.push(format!(
                "{label}: {status} with tenant B's content in the body"
            ));
        }
        if matches!(status.as_u16(), 400 | 413 | 415 | 422) {
            body_rejected += 1;
        }
    }
    println!(
        "probed {} operations; {body_rejected} refused at the body; parameters not mapped: {unknown:?}",
        probes.len()
    );
    assert!(
        leaks.is_empty(),
        "workspace A reached workspace B:\n{}",
        leaks.join("\n")
    );

    // Nothing A sent changed B.
    let msg: Value = h
        .send(
            Method::GET,
            &format!("/messages/{}", b.message),
            &b.bearer,
            None,
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(msg["body"], SECRET_BODY, "B's message changed: {msg}");
    for path in [
        format!("/threads/{}", b.thread),
        format!("/channels/{}", b.channel),
        format!("/workspaces/{}", b.workspace),
    ] {
        let res = h.send(Method::GET, &path, &b.bearer, None).await;
        assert!(res.status().is_success(), "{path}: {}", res.status());
    }
    let members: Value = h
        .send(
            Method::GET,
            &format!("/workspaces/{}/members", b.workspace),
            &b.bearer,
            None,
        )
        .await
        .json()
        .await
        .unwrap();
    assert!(
        members.to_string().contains(&b.member),
        "B's member is gone: {members}"
    );
}
