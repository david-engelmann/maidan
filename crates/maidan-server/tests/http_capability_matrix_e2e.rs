//! Table-driven HTTP capability denial from `contracts/http-capability-map.json` (Cluster 77).

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

#[derive(Debug, serde::Deserialize)]
struct MapEntry {
    method: String,
    path: String,
    capability: String,
    surface: String,
}

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

struct FixtureIds {
    workspace: String,
    member: String,
    channel: String,
    thread: String,
    message: String,
    sha: String,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn() -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        server,
        client: reqwest::Client::new(),
        store,
        _dir: dir,
    }
}

async fn mint_token(
    store: &dyn Store,
    workspace_id: maidan_types::WorkspaceId,
    member_id: maidan_types::MemberId,
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

fn setup_caps() -> Vec<String> {
    vec![
        capability::WORKSPACE_READ.into(),
        capability::WORKSPACE_WRITE.into(),
        capability::MESSAGE_POST.into(),
        capability::THREAD_TRANSITION.into(),
        capability::ARTIFACT_UPLOAD.into(),
        capability::SEARCH_QUERY.into(),
        capability::TOKEN_ADMIN.into(),
        capability::FEDERATION_ADMIN.into(),
    ]
}

fn http_deny_caps(required: &str) -> Vec<String> {
    match required {
        capability::WORKSPACE_READ => vec![],
        capability::WORKSPACE_WRITE => vec![capability::WORKSPACE_READ.into()],
        capability::MESSAGE_POST => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        capability::THREAD_TRANSITION => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        capability::ARTIFACT_UPLOAD => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        capability::SEARCH_QUERY => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        capability::TOKEN_ADMIN => vec![capability::WORKSPACE_READ.into()],
        capability::FEDERATION_ADMIN => vec![capability::WORKSPACE_READ.into()],
        capability::AUDIT_READ_GLOBAL => vec![capability::WORKSPACE_READ.into()],
        capability::CHANNEL_ADMIN => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        other => panic!("unsupported capability in http map: {other}"),
    }
}

fn load_http_map() -> Vec<MapEntry> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/http-capability-map.json");
    serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("http capability map json")
}

fn substitute_path(template: &str, f: &FixtureIds) -> String {
    if template.starts_with("/workspaces/") {
        let delivery_id = if template.contains("/deliveries/{did}") {
            "1"
        } else {
            f.workspace.as_str()
        };
        return template
            .replace("{wid}", &f.workspace)
            .replace("{id}", &f.workspace)
            .replace("{mid}", &f.member)
            .replace("{app_id}", &f.workspace)
            .replace("{iid}", &f.workspace)
            .replace("{oid}", &f.workspace)
            .replace("{pid}", &f.workspace)
            .replace("{whid}", &f.workspace)
            .replace("{cid}", &f.channel)
            .replace("{hid}", &f.workspace)
            .replace("{did}", delivery_id);
    }
    if template.starts_with("/ui/api/workspaces/") {
        return template
            .replace("{wid}", &f.workspace)
            .replace("{cid}", &f.channel)
            .replace("{tid}", &f.thread)
            .replace("{mid}", &f.message);
    }
    if template.starts_with("/ui/api/channels/") {
        return template.replace("{cid}", &f.channel);
    }
    if template.starts_with("/ui/api/threads/") {
        return template.replace("{tid}", &f.thread);
    }
    if template.starts_with("/ui/api/messages/") {
        return template.replace("{mid}", &f.message);
    }
    if template.starts_with("/members/") {
        return template.replace("{id}", &f.member);
    }
    if template.starts_with("/channels/") {
        return template
            .replace("{cid}", &f.channel)
            .replace("{mid}", &f.member)
            .replace("{id}", &f.channel);
    }
    if template.starts_with("/threads/") {
        return template
            .replace("{tid}", &f.thread)
            .replace("{dep_id}", &f.thread)
            .replace("{id}", &f.thread);
    }
    if template.starts_with("/messages/") {
        return template.replace("{id}", &f.message);
    }
    if template.starts_with("/artifacts/") {
        return template
            .replace("{sha}", &f.sha)
            .replace("{upload_id}", &f.workspace)
            .replace("{part_number}", "1");
    }
    if template.starts_with("/dm/") {
        return template.replace("{id}", &f.workspace);
    }
    if template.starts_with("/tokens/") {
        return template.replace("{id}", &f.workspace);
    }
    if template.starts_with("/references") {
        return template.to_string();
    }
    if template.starts_with("/operator/") {
        return template.replace("{job_id}", &f.workspace);
    }
    if template.starts_with("/task-schedules/") {
        // Any UUID works — the cap() check 403s before the id is looked up.
        return template.replace("{id}", &f.workspace);
    }
    template.to_string()
}

fn apply_route_defaults(
    builder: reqwest::RequestBuilder,
    method: &str,
    path: &str,
    f: &FixtureIds,
) -> reqwest::RequestBuilder {
    let mut b = builder;
    if path.contains("/search") {
        b = b.query(&[("q", "hello")]);
    }
    if path == "/references" && method == "GET" {
        b = b.query(&[("src_kind", "message"), ("src_id", f.message.as_str())]);
    }
    if path.ends_with("/dm") && method == "GET" {
        b = b.query(&[("member_id", f.member.as_str())]);
    }
    if path.ends_with("/dm") && method == "POST" {
        return b.json(&json!({
            "member_id": f.member,
            "other_member_id": f.workspace,
        }));
    }
    if path == "/workspaces/{id}" && method == "DELETE" {
        return b.json(&json!({ "confirm_workspace_id": f.workspace }));
    }
    if path.contains("/purge") && method == "POST" {
        return b.json(&json!({ "confirm_workspace_id": f.workspace }));
    }
    if path.contains("/inbox/read") {
        return b.json(&json!({ "read_through": "2026-01-01T00:00:00Z" }));
    }
    if path.contains("/deliveries/{did}") {
        b = b.query(&[("kind", "automation")]);
    }
    if path.contains("/members/") && path.ends_with("/tokens") && method == "POST" {
        return b.json(&json!({
            "capabilities": [capability::WORKSPACE_READ],
            "label": "deny-matrix"
        }));
    }
    if path.ends_with("/mention-webhook") && method == "PUT" {
        return b.json(&json!({ "webhook_id": null }));
    }
    if path.contains("/reactions") && method == "DELETE" {
        return b.json(&json!({
            "member_id": f.member,
            "emoji": "thumbsup"
        }));
    }
    if path.contains("/reactions") && method == "POST" {
        return b.json(&json!({ "emoji": "thumbsup", "member_id": f.member }));
    }
    if path.contains("/votes") && method == "POST" {
        return b.json(&json!({ "member_id": f.member, "kind": "upvote" }));
    }
    if path.contains("/mentions") && method == "POST" {
        return b.json(&json!({ "member_id": f.member }));
    }
    if path.ends_with("/members") && method == "POST" {
        return b.json(&json!({ "member_id": f.member }));
    }
    if path.contains("/pins") && (method == "POST" || method == "DELETE") {
        return b.json(&json!({
            "message_id": f.message,
            "member_id": f.member
        }));
    }
    if path == "/threads/{id}" && method == "POST" {
        return b.json(&json!({
            "actor_id": f.member,
            "action": "start_review"
        }));
    }
    if path == "/threads/{id}/assignee" && method == "PUT" {
        return b.json(&json!({
            "actor_id": f.member,
            "assignee_id": f.member
        }));
    }
    if path == "/threads/{id}/assignee" && method == "DELETE" {
        return b.json(&json!({ "actor_id": f.member }));
    }
    if path == "/threads/{id}/assignee/claim" && method == "POST" {
        return b.json(&json!({ "member_id": f.member }));
    }
    if path == "/channels/{cid}/threads/claim-next" && method == "POST" {
        return b.json(&json!({ "member_id": f.member }));
    }
    if path == "/threads/{id}/claim/renew" && method == "POST" {
        return b.json(&json!({ "member_id": f.member, "lease_secs": 60 }));
    }
    if path == "/threads/{id}/dependencies" && method == "POST" {
        return b.json(&json!({ "depends_on_thread_id": f.thread }));
    }
    if path == "/workspaces/{wid}/task-schedules" && method == "POST" {
        return b.json(&json!({ "channel_id": f.channel, "title": "cap matrix" }));
    }
    if path == "/task-schedules/{id}" && method == "PUT" {
        return b.json(&json!({ "active": false }));
    }
    if path.ends_with("/messages") && method == "POST" {
        return b.json(&json!({
            "author_id": f.member,
            "body": "cap matrix"
        }));
    }
    if path.contains("/messages/") && method == "PATCH" {
        return b.json(&json!({
            "editor_id": f.member,
            "body": "edited"
        }));
    }
    if path == "/references" && method == "POST" {
        return b.json(&json!({
            "src_kind": "message",
            "src_id": f.message,
            "dst_kind": "message",
            "dst_id": f.message,
            "relation": "related"
        }));
    }
    if path.ends_with("/apps") && method == "POST" && path.contains("/workspaces/") {
        return b.json(&json!({
            "slug": "cap-matrix-app",
            "name": "Cap Matrix App"
        }));
    }
    if path.contains("/peers") && method == "POST" {
        return b.json(&json!({ "name": "peer", "base_url": "https://peer.example" }));
    }
    if path.contains("/webhooks") && method == "POST" {
        return b.json(&json!({
            "url": "https://example.com/hook",
            "event_kinds": ["message.posted"]
        }));
    }
    if path.contains("/slash-commands") && method == "POST" {
        return b.json(&json!({
            "name": "cap",
            "handler_kind": "http",
            "handler_target": "https://example.com/slash"
        }));
    }
    if path.contains("/fsm-hooks") && method == "POST" {
        return b.json(&json!({
            "to_state": "closed",
            "handler_kind": "http",
            "handler_target": "https://example.com/fsm"
        }));
    }
    if path == "/operator/reindex-embeddings" && method == "POST" {
        return b.json(&json!({}));
    }
    if path.contains("/channels") && method == "POST" && path.contains("/workspaces/") {
        return b.json(&json!({ "name": "cap-matrix" }));
    }
    if path == "/artifacts" && method == "POST" {
        return b.query(&[("kind", "attachment")]).body("bytes");
    }
    if method == "POST" || method == "PUT" || method == "PATCH" {
        b.json(&json!({}))
    } else {
        b
    }
}

async fn seed_fixture(
    h: &Harness,
) -> (
    FixtureIds,
    maidan_types::WorkspaceId,
    maidan_types::MemberId,
) {
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "http-cap-matrix".into(),
        })
        .await
        .unwrap();
    let member = h
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let bearer = mint_token(h.store.as_ref(), ws.id, member.id, setup_caps()).await;
    let workspace = ws.id.0.to_string();
    let member_id = member.id.0.to_string();
    let base = h.base();

    let ch: Value = h
        .client
        .post(format!("{base}/workspaces/{workspace}/channels"))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({ "name": "general" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel = ch["id"].as_str().unwrap().to_string();

    let th: Value = h
        .client
        .post(format!("{base}/channels/{channel}/threads"))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread = th["id"].as_str().unwrap().to_string();

    let msg: Value = h
        .client
        .post(format!("{base}/threads/{thread}/messages"))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({
            "author_id": member_id,
            "body": "seed"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let message = msg["id"].as_str().unwrap().to_string();

    let fixture = FixtureIds {
        workspace: workspace.clone(),
        member: member_id,
        channel,
        thread,
        message,
        sha: "a".repeat(64),
    };
    (fixture, ws.id, member.id)
}

fn should_skip(entry: &MapEntry) -> bool {
    entry.surface != "http"
        || entry.capability == "per-tool"
        || entry.capability == "per-rpc"
        || entry.path == "/a2a/v1/events"
        || entry.path.contains("/artifacts/multipart")
        || entry.path.contains("/apps")
        || entry.path.contains("/app-installations")
        || entry.path.contains("/peers")
        || entry.path.contains("/automation/deliveries/{did}")
        || entry.path.contains("/outbox/{oid}/replay")
        || entry.path.contains("/oauth/")
}

#[tokio::test]
async fn every_http_map_route_denies_without_required_capability() {
    let map = load_http_map();
    let h = spawn().await;
    let (fixture, workspace_id, member_id) = seed_fixture(&h).await;

    let mut exercised = 0usize;
    for entry in map {
        if should_skip(&entry) {
            continue;
        }
        let bearer = mint_token(
            h.store.as_ref(),
            workspace_id,
            member_id,
            http_deny_caps(&entry.capability),
        )
        .await;
        let url = format!("{}{}", h.base(), substitute_path(&entry.path, &fixture));
        let builder = match entry.method.as_str() {
            "GET" => h.client.get(&url),
            "POST" => h.client.post(&url),
            "PUT" => h.client.put(&url),
            "PATCH" => h.client.patch(&url),
            "DELETE" => h.client.delete(&url),
            other => panic!("unsupported method {other}"),
        };
        let req = apply_route_defaults(
            builder.header("Authorization", format!("Bearer {bearer}")),
            &entry.method,
            &entry.path,
            &fixture,
        );
        let resp = req.send().await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{} {} ({})",
            entry.method,
            entry.path,
            entry.capability
        );
        exercised += 1;
    }
    assert!(
        exercised >= 60,
        "expected broad http map coverage, got {exercised} routes"
    );

    h.shutdown().await;
}
