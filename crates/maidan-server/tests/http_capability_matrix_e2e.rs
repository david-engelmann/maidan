//! Table-driven HTTP capability denial from
//! `contracts/http-capability-map.json`.
//!
//! Each mapped route is called with a token lacking the mapped capability and
//! must answer 403 naming that capability ("missing capability: X"). The
//! name check is what ties the map to the code: a bare 403 was once enough,
//! so when instance-wide reindex moved to `operator:global` the map kept
//! saying `token:admin` and still passed (the deny token lacked both).

use maidan_store::test_support::SetResultClaiming;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
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
    /// See `http_openapi_capability_map_contract.rs`: routes gated on the
    /// caller's app identity rather than a member capability.
    #[serde(default)]
    gate: Option<String>,
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
    /// An instance-wide reindex job, so `GET .../{job_id}` reaches the gate
    /// a real job has rather than the unknown-id branch.
    reindex_job: String,
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
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
        // The deny-token holds everything EXCEPT operator:global, so a route
        // gated on it must still 403 — which is the whole point of introducing
        // it rather than reusing the per-workspace token:admin.
        capability::OPERATOR_GLOBAL => vec![
            capability::WORKSPACE_READ.into(),
            capability::TOKEN_ADMIN.into(),
        ],
        capability::CHANNEL_ADMIN => vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
        capability::SECRET_READ => vec![capability::WORKSPACE_READ.into()],
        capability::SECRET_ADMIN => vec![capability::WORKSPACE_READ.into()],
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
            .replace("{hold_id}", &f.workspace)
            .replace("{oid}", &f.workspace)
            .replace("{pid}", &f.workspace)
            .replace("{whid}", &f.workspace)
            .replace("{cid}", &f.channel)
            .replace("{hid}", &f.workspace)
            .replace("{gid}", &f.workspace)
            .replace("{tid}", &f.workspace)
            .replace("{term}", "testterm")
            .replace("{name}", "capsecret")
            .replace("{host}", "hooks.example.com")
            .replace("{slack_channel_id}", "CTEST")
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
        return template
            .replace("{id}", &f.member)
            .replace("{nid}", &f.member)
            .replace("{sub_id}", &f.member)
            .replace("{followed_id}", &f.member)
            .replace("{cid}", &f.channel)
            .replace("{tid}", &f.thread)
            .replace("{skill}", "testskill");
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
            .replace("{block_id}", &f.thread)
            .replace("{member_id}", &f.member)
            .replace("{skill}", "testskill")
            .replace("{did}", &f.workspace)
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
    if template.starts_with("/group-dms/") {
        return template.replace("{id}", &f.workspace);
    }
    if template.starts_with("/tokens/") {
        return template.replace("{id}", &f.workspace);
    }
    if template.starts_with("/references") {
        return template.to_string();
    }
    if template.starts_with("/operator/") {
        // {job_id} is a seeded instance-wide job; any UUID works for {id}
        // (cap() 403s before the id is looked up).
        return template
            .replace("{job_id}", &f.reindex_job)
            .replace("{id}", &f.workspace);
    }
    if template.starts_with("/task-schedules/") {
        // Any UUID works — the cap() check 403s before the id is looked up.
        return template.replace("{id}", &f.workspace);
    }
    if template.starts_with("/approval-gates/") {
        // Any UUID works — cap() 403s before the gate is looked up.
        return template.replace("{id}", &f.workspace);
    }
    template.to_string()
}

/// Shape-valid signed envelope so `ApiJson<SignedExport>` deserializes and
/// `cap()` is what the matrix exercises (verify fails after a capable token).
fn signed_export_stub() -> Value {
    json!({
        "$type": "maidan.workspace.export/1",
        "alg": "ed25519",
        "token_policy": "tokens_die_on_export",
        "public_key": "11".repeat(32),
        "signed_at": "2026-01-01T00:00:00Z",
        "payload": {
            "format_version": 1,
            "exported_at": "2026-01-01T00:00:00Z",
            "workspace": {
                "id": "00000000-0000-0000-0000-000000000001",
                "name": "x",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "tombstoned_at": null
            },
            "members": [],
            "channels": [],
            "threads": [],
            "messages": [],
            "message_edits": [],
            "pins": [],
            "references": []
        },
        "content_sha256": "00".repeat(32),
        "signature": "00".repeat(64)
    })
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
    if path == "/workspaces/{id}/slots" && method == "DELETE" {
        b = b.query(&[("slot", "rust-build")]);
    }
    if path == "/workspaces/{id}/repo-pauses" && method == "DELETE" {
        b = b.query(&[("repo", "example/app")]);
    }
    if path.ends_with("/dm") && method == "GET" {
        b = b.query(&[("member_id", f.member.as_str())]);
    }
    if path.ends_with("/dm") && method == "POST" {
        return b.json(&json!({
            "other_member_id": f.workspace,
        }));
    }
    if path.ends_with("/group-dms") && method == "GET" {
        b = b.query(&[("member_id", f.member.as_str())]);
    }
    if path.ends_with("/group-dms") && method == "POST" {
        return b.json(&json!({
            "member_ids": [f.member, f.workspace],
            "title": "cap matrix",
        }));
    }
    if path.ends_with("/group-dms/{id}/messages") && method == "POST" {
        return b.json(&json!({
            "body": "cap matrix",
        }));
    }
    if path == "/workspaces/{id}" && method == "DELETE" {
        return b.json(&json!({ "confirm_workspace_id": f.workspace }));
    }
    if path.contains("/purge") && method == "POST" {
        return b.json(&json!({ "confirm_workspace_id": f.workspace }));
    }
    if path == "/workspaces/import" && method == "POST" {
        return b.json(&signed_export_stub());
    }
    if path == "/workspaces/export/verify" && method == "POST" {
        return b.json(&signed_export_stub());
    }
    if path.contains("/inbox/read") {
        return b.json(&json!({ "read_through": "2026-01-01T00:00:00Z" }));
    }
    if path.ends_with("/snooze") && method == "POST" {
        return b.json(&json!({ "until": "2026-01-01T00:00:00Z" }));
    }
    if path.ends_with("/notification-prefs") && method == "PUT" {
        return b.json(&json!({ "kind": "mention_recorded", "muted": true }));
    }
    if path.ends_with("/channel-follows") && method == "POST" {
        return b.json(&json!({ "channel_id": f.channel }));
    }
    if path.ends_with("/thread-follows") && method == "POST" {
        return b.json(&json!({ "thread_id": f.thread }));
    }
    if path.ends_with("/member-follows") && method == "POST" {
        return b.json(&json!({ "followed_member_id": f.member }));
    }
    if path.ends_with("/email") && method == "PUT" {
        return b.json(&json!({ "email": "user@example.com" }));
    }
    if path.ends_with("/delivery-mode") && method == "PUT" {
        return b.json(&json!({ "mode": "digest" }));
    }
    if path.ends_with("/wip-limit") && method == "PUT" {
        return b.json(&json!({ "limit": 5 }));
    }
    if path.ends_with("/approval-policy") && method == "PUT" {
        return b.json(&json!({ "confirm_at": "medium" }));
    }
    if path.ends_with("/delegation-policy") && method == "PUT" {
        return b.json(&json!({ "max_grant_days": 30 }));
    }
    if path.ends_with("/retention") && method == "PUT" {
        return b.json(&json!({ "messages_days": 30 }));
    }
    if path.ends_with("/spawn-budget") && method == "PUT" {
        return b.json(&json!({ "max_children": 4, "max_depth": 3, "max_tools": 64 }));
    }
    if path.ends_with("/cancel") && method == "POST" {
        return b.json(&json!({ "reason": "cap matrix" }));
    }
    if path.ends_with("/unclaimable") && method == "PUT" {
        return b.json(&json!({ "reason": "cap matrix" }));
    }
    if path.ends_with("/block") && method == "PUT" {
        return b.json(&json!({ "reason": "gate" }));
    }
    if path.ends_with("/wait") && method == "PUT" {
        return b.json(&json!({ "wait_until": "2099-01-01T00:00:00Z" }));
    }
    if path.ends_with("/priority") && method == "PUT" {
        return b.json(&json!({ "priority": 5 }));
    }
    if path.ends_with("/legal-holds") && method == "POST" {
        return b.json(&json!({ "reason": "cap matrix" }));
    }
    if path.contains("/glossary/") && method == "PUT" {
        return b.json(&json!({ "definition": "cap matrix" }));
    }
    if path.ends_with("/seed") && method == "POST" {
        return b.json(&json!({ "title": "cap matrix" }));
    }
    if path.contains("/deliveries/{did}") {
        b = b.query(&[("kind", "automation")]);
    }
    if path == "/workspaces/{wid}/agents" && method == "POST" {
        return b.json(&json!({ "handle": "deny-matrix-agent" }));
    }
    if path.contains("/members/") && path.ends_with("/tokens") && method == "POST" {
        return b.json(&json!({
            "capabilities": [capability::WORKSPACE_READ],
            "label": "deny-matrix"
        }));
    }
    if path == "/tokens/attenuate" && method == "POST" {
        return b.json(&json!({
            "capabilities": [capability::WORKSPACE_READ],
            "label": "deny-matrix"
        }));
    }
    if path == "/tokens/delegate" && method == "POST" {
        return b.json(&json!({
            "grant_id": f.workspace,
            "capabilities": [capability::WORKSPACE_READ]
        }));
    }
    if path == "/workspaces/{wid}/delegation-grants" && method == "POST" {
        return b.json(&json!({
            "subject_id": f.member,
            "delegate_id": f.workspace,
            "capabilities": [capability::WORKSPACE_READ],
            "purpose": "capability matrix",
            "expires_at": "2099-01-01T00:00:00Z"
        }));
    }
    if path.ends_with("/handle") && method == "PUT" {
        return b.json(&json!({ "handle": "cap-matrix" }));
    }
    if path.ends_with("/mention-webhook") && method == "PUT" {
        return b.json(&json!({ "webhook_id": null }));
    }
    if path.contains("/reactions") && method == "DELETE" {
        return b.json(&json!({ "emoji": "thumbsup" }));
    }
    if path.contains("/reactions") && method == "POST" {
        return b.json(&json!({ "emoji": "thumbsup" }));
    }
    if path.contains("/votes") && method == "DELETE" {
        return b.json(&json!({ "kind": "approve" }));
    }
    if path.contains("/votes") && method == "POST" {
        return b.json(&json!({ "kind": "approve" }));
    }
    if path.contains("/mentions") && method == "POST" {
        return b.json(&json!({ "member_id": f.member }));
    }
    if path.ends_with("/members") && method == "POST" {
        return b.json(&json!({ "member_id": f.member }));
    }
    if path.contains("/pins") && (method == "POST" || method == "DELETE") {
        return b.json(&json!({ "message_id": f.message }));
    }
    if path == "/threads/{id}" && method == "POST" {
        return b.json(&json!({
            "action": "start_review"
        }));
    }
    if path == "/threads/{id}/assignee" && method == "PUT" {
        return b.json(&json!({
            "assignee_id": f.member
        }));
    }
    if path == "/threads/{id}/assignee" && method == "DELETE" {
        return b.json(&json!({}));
    }
    if path == "/threads/{id}/owner" && method == "PUT" {
        return b.json(&json!({ "owner_id": f.member }));
    }
    if path == "/threads/{id}/title" && method == "PUT" {
        return b.json(&json!({ "title": "cap matrix rename" }));
    }
    // A PUT replaces the whole envelope, so every dimension must be stated —
    // `null` where there is no cap. The capability check runs first either way,
    // but a partial body here would document a request shape the API refuses.
    if path == "/threads/{id}/budget" && method == "PUT" {
        return b.json(&json!({
            "max_tokens": 1000,
            "max_usd_micros": null,
            "max_turns": null,
            "max_wall_secs": null
        }));
    }
    // A PATCH names only what it changes.
    if path == "/threads/{id}/budget" && method == "PATCH" {
        return b.json(&json!({ "max_tokens": 1000 }));
    }
    if path == "/threads/{id}/usage/otel" && method == "POST" {
        return b.json(&json!({
            "usage_report_id": uuid::Uuid::new_v4(),
            "claim_lease_id": uuid::Uuid::nil(),
            "attributes": {
                "gen_ai.response.model": "capability-matrix/test",
                "gen_ai.usage.input_tokens": 1,
                "gen_ai.usage.output_tokens": 0
            },
            "price_snapshot": {
                "input_usd_micros_per_million": 0,
                "output_usd_micros_per_million": 0,
                "cache_read_usd_micros_per_million": 0,
                "cache_write_5m_usd_micros_per_million": 0,
                "cache_write_1h_usd_micros_per_million": 0
            }
        }));
    }
    if path == "/threads/{id}/usage" && method == "POST" {
        return b.json(&json!({
            "usage_report_id": uuid::Uuid::new_v4(),
            "claim_lease_id": uuid::Uuid::nil(),
            "model": "capability-matrix/test",
            "tokens": {
                "input": 1,
                "output": 0,
                "cache_read": 0,
                "cache_write_5m": 0,
                "cache_write_1h": 0
            },
            "usd_micros": 0,
            "price_snapshot": {
                "input_usd_micros_per_million": 0,
                "output_usd_micros_per_million": 0,
                "cache_read_usd_micros_per_million": 0,
                "cache_write_5m_usd_micros_per_million": 0,
                "cache_write_1h_usd_micros_per_million": 0
            },
            "turns": 1
        }));
    }
    if path == "/threads/{id}/assignee/claim" && method == "POST" {
        return b.json(&json!({}));
    }
    if path == "/channels/{cid}/threads/claim-next" && method == "POST" {
        return b.json(&json!({}));
    }
    if path == "/workspaces/{wid}/threads/claim-next" && method == "POST" {
        return b.json(&json!({}));
    }
    if path == "/threads/{id}/claim/renew" && method == "POST" {
        return b.json(&json!({
            "claim_lease_id": uuid::Uuid::nil(),
            "lease_secs": 60
        }));
    }
    if path == "/threads/{id}/claim/acknowledge" && method == "POST" {
        return b.json(&json!({
            "claim_lease_id": uuid::Uuid::nil()
        }));
    }
    if path == "/threads/{id}/claim/release" && method == "POST" {
        return b.json(&json!({
            "claim_lease_id": uuid::Uuid::nil()
        }));
    }
    if path == "/threads/{id}/dependencies" && method == "POST" {
        return b.json(&json!({ "depends_on_thread_id": f.thread }));
    }
    if path == "/threads/{id}/result" && method == "PUT" {
        return b.json(&json!({ "result": { "ok": true }, "claim_lease_id": uuid::Uuid::nil() }));
    }
    if path == "/operator/workspaces/{id}/suspension" && method == "PUT" {
        return b.json(&json!({ "reason": "probe" }));
    }
    if path == "/threads/{id}/pull" && method == "PUT" {
        return b.json(&json!({ "repo": "example/app", "pull_number": 1 }));
    }
    if path == "/workspaces/{id}/pull-watches" && method == "POST" {
        return b.json(&json!({ "repo": "example/app", "pull_number": 1 }));
    }
    if path == "/workspaces/{id}/linear-claim-comments" && method == "PUT" {
        return b.json(&json!({ "enabled": false }));
    }
    if path == "/workspaces/{id}/default-engine" && method == "PUT" {
        return b.json(&json!({ "engine": null }));
    }
    if path == "/workspaces/{id}/slots" && method == "PUT" {
        return b.json(&json!({ "slot": "rust-build", "capacity": 2 }));
    }
    if path == "/workspaces/{id}/repo-pauses" && method == "PUT" {
        return b.json(&json!({ "repo": "example/app" }));
    }
    if path == "/threads/{id}/plan" && method == "POST" {
        return b.json(&json!({
            "lease_id": "00000000-0000-0000-0000-000000000000",
            "children": [{ "key": "a", "title": "a" }]
        }));
    }
    if path == "/threads/{id}/plan/amend" && method == "POST" {
        return b.json(&json!({
            "lease_id": "00000000-0000-0000-0000-000000000000",
            "add": [{ "key": "b", "title": "b" }],
            "reason": "the plan missed a step"
        }));
    }
    if path == "/threads/{id}/attributes" && method == "PUT" {
        return b.json(&json!({
            "kind": "review",
            "engine": "pi",
            "threat_model": true
        }));
    }
    if path == "/threads/{id}/footprint" && method == "PUT" {
        return b.json(&json!({
            "repo": "example/app",
            "mode": "exclusive",
            "items": [{ "kind": "lane", "value": "bridge" }]
        }));
    }
    if path == "/threads/{id}/steer" && method == "PUT" {
        return b.json(&json!({ "steer": "focus on the failing test first" }));
    }
    if path == "/threads/{id}/lineage" && method == "PUT" {
        return b.json(&json!({ "parent_run_id": "aa4dc966-0e09-44c3-b7a5-2d048b48b301" }));
    }
    if path == "/approval-gates/{id}/answer" && method == "POST" {
        return b.json(&json!({ "request_state": "x", "action": "accept" }));
    }
    if path == "/workspaces/{wid}/task-schedules" && method == "POST" {
        return b.json(&json!({ "channel_id": f.channel, "title": "cap matrix" }));
    }
    if path == "/workspaces/{wid}/recipes" && method == "POST" {
        return b.json(&json!({ "channel_id": f.channel, "name": "cap matrix", "spec": {} }));
    }
    if path == "/workspaces/{wid}/recipes/{id}/instantiate" && method == "POST" {
        return b.json(&json!({ "params": {} }));
    }
    if path == "/workspaces/{wid}/secrets" && method == "POST" {
        return b.json(&json!({ "name": "capsecret", "value": "v" }));
    }
    if path == "/workspaces/{wid}/secret-egress-hosts" && method == "POST" {
        return b.json(&json!({ "host": "hooks.example.com" }));
    }
    if path == "/workspaces/{wid}/share-tickets" && method == "POST" {
        return b.json(&json!({
            "channel_id": f.channel,
            "expires_at": "2099-01-01T00:00:00Z",
            "artifact_shas": []
        }));
    }
    if path == "/members/{id}/freeze" && method == "POST" {
        return b.json(&json!({}));
    }
    if path == "/workspaces/{wid}/memory-blocks" && method == "POST" {
        return b.json(&json!({ "label": "cap-matrix" }));
    }
    if path == "/workspaces/{wid}/memory-blocks/{id}" && method == "PUT" {
        return b.json(&json!({ "value": "cap matrix" }));
    }
    if path == "/threads/{id}/review-requirement" && method == "PUT" {
        return b.json(&json!({ "required_count": 1 }));
    }
    if path == "/threads/{id}/reviewers" && method == "POST" {
        return b.json(&json!({ "member_id": f.member }));
    }
    if path == "/threads/{id}/reviews" && method == "POST" {
        return b.json(&json!({ "decision": "approve" }));
    }
    if path == "/threads/{id}/land-gate" && method == "PUT" {
        return b.json(&json!({ "status": "pass" }));
    }
    if path == "/threads/{id}/land-gate/advice" && method == "POST" {
        return b.json(&json!({ "state": "capability matrix" }));
    }
    if path == "/task-schedules/{id}" && method == "PUT" {
        return b.json(&json!({ "active": false }));
    }
    if path.ends_with("/skills") && method == "POST" && path.contains("/members/") {
        return b.json(&json!({ "skill": "cap-matrix" }));
    }
    if path.ends_with("/push-subscriptions") && method == "POST" {
        return b.json(&json!({
            "endpoint": "https://push.example.com/x",
            "keys": { "p256dh": "x", "auth": "y" }
        }));
    }
    if path.ends_with("/required-skills") && method == "POST" {
        return b.json(&json!({ "skill": "cap-matrix" }));
    }
    if path.ends_with("/messages") && method == "POST" {
        return b.json(&json!({
            "body": "cap matrix"
        }));
    }
    if path.contains("/messages/") && method == "PATCH" {
        return b.json(&json!({
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
    // No body: the grant being revoked is named in the path. The clause
    // exists so this probe's shape is chosen here, not inherited from the
    // default fallthrough below.
    if path.contains("/oauth-grants/") && method == "POST" {
        return b.json(&json!({}));
    }
    // A valid selector, so the extractor passes and it is `cap()` that answers.
    if path.contains("/egress-targets") && method == "POST" {
        return b.json(&json!({ "surface": "slack", "selector": "C0123ABCDEF" }));
    }
    if path.contains("/slack-links") && method == "POST" {
        return b.json(&json!({
            "slack_channel_id": "CTEST",
            "thread_id": "00000000-0000-0000-0000-000000000000"
        }));
    }
    if path.contains("/github-links") && method == "POST" {
        return b.json(&json!({
            "repo": "owner/name",
            "issue_number": 1,
            "thread_id": "00000000-0000-0000-0000-000000000000"
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
    // A JSON-RPC body, so a later probe is not refused by the parser before
    // the capability check. The matrix skips the MCP surface today.
    if matches!(
        path,
        "/mcp" | "/mcp/worker" | "/mcp/reviewer" | "/mcp/streamable"
    ) && method == "POST"
    {
        return b.json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }));
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
            "body": "seed"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let message = msg["id"].as_str().unwrap().to_string();

    let reindex_job = uuid::Uuid::now_v7();
    h.store
        .upsert_reindex_job(maidan_types::ReindexJob {
            job_id: reindex_job,
            status: maidan_types::ReindexJobStatus::Completed,
            workspace_id: None,
            embedding_model: "hash-v1".into(),
            processed: Some(0),
            failed: Some(0),
            error: None,
            started_at: chrono::Utc::now(),
            finished_at: Some(chrono::Utc::now()),
        })
        .await
        .unwrap();

    let fixture = FixtureIds {
        workspace: workspace.clone(),
        member: member_id,
        channel,
        thread,
        message,
        sha: "a".repeat(64),
        reindex_job: reindex_job.to_string(),
    };
    (fixture, ws.id, member.id)
}

/// How a capability refusal names what was missing.
fn named(capability: &str) -> String {
    format!("missing capability: {capability}")
}

fn should_skip(entry: &MapEntry) -> bool {
    entry.surface != "http"
        || entry.capability == "per-tool"
        || entry.capability == "per-rpc"
        // App-installation gates are proven by their own e2e (for mark-ready,
        // `mark_ready_e2e::only_the_mark_ready_app_may_mark_ready` asserts the
        // 403 names the app); the deny-a-capability probe cannot exercise them.
        || entry.gate.as_deref() == Some("app-installation")
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
    let mut failures = Vec::new();
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
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status != StatusCode::FORBIDDEN {
            failures.push(format!(
                "{} {} ({}): {status}, not 403",
                entry.method, entry.path, entry.capability
            ));
        } else if !body.contains(&named(&entry.capability)) {
            // A 403 alone proves only that the token lacked *something*. The
            // refusal must name the capability the map claims, or the map is
            // stale (the route needs another one, perhaps a stronger one).
            failures.push(format!(
                "{} {} maps to {} but is refused for: {body}",
                entry.method, entry.path, entry.capability
            ));
        }
        exercised += 1;
    }
    assert!(
        failures.is_empty(),
        "http-capability-map does not match what the routes enforce:\n{}",
        failures.join("\n")
    );
    assert!(
        exercised >= 60,
        "expected broad http map coverage, got {exercised} routes"
    );

    h.shutdown().await;
}

// Behavior for routes no other test calls. The table above proves each one
// asks for its capability; these prove what each answers to a caller that has
// it, and that a second workspace's caller, holding the same capabilities,
// gets none of the first workspace's rows.

/// One workspace with a work channel, a thread, and two members, each with a
/// token holding `setup_caps()`: `admin` reads and administers, `agent` claims
/// and reports.
struct Tenant {
    workspace: maidan_types::WorkspaceId,
    channel: maidan_types::ChannelId,
    thread: maidan_types::ThreadId,
    admin: maidan_types::MemberId,
    admin_token: String,
    agent: maidan_types::MemberId,
    agent_token: String,
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let workspace = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let mut members = Vec::new();
    for (handle, kind) in [("admin", MemberKind::Human), ("agent", MemberKind::Agent)] {
        let id = store
            .create_member(NewMember {
                workspace_id: workspace,
                handle: handle.into(),
                display_name: None,
                kind,
            })
            .await
            .unwrap()
            .id;
        members.push((id, mint_token(store, workspace, id, setup_caps()).await));
    }
    let channel = store
        .create_channel(maidan_types::NewChannel {
            workspace_id: workspace,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id;
    let thread = new_thread(store, channel, "first").await;
    let (agent, agent_token) = members.pop().unwrap();
    let (admin, admin_token) = members.pop().unwrap();
    Tenant {
        workspace,
        channel,
        thread,
        admin,
        admin_token,
        agent,
        agent_token,
    }
}

async fn new_thread(
    store: &dyn Store,
    channel: maidan_types::ChannelId,
    title: &str,
) -> maidan_types::ThreadId {
    store
        .create_thread(maidan_types::NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

async fn call(
    h: &Harness,
    method: reqwest::Method,
    path: &str,
    token: &str,
) -> (StatusCode, Value) {
    let resp = h
        .client
        .request(method, format!("{}{path}", h.base()))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn read(h: &Harness, path: &str, token: &str) -> (StatusCode, Value) {
    call(h, reqwest::Method::GET, path, token).await
}

/// Another workspace's caller is refused without learning anything: a 403 or
/// a 404, and no id from the refused workspace in the body.
fn assert_refused(what: &str, (status, body): &(StatusCode, Value), secrets: &[String]) {
    assert!(
        matches!(*status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "{what}: another workspace got {status} {body}"
    );
    let text = body.to_string();
    for secret in secrets {
        assert!(!text.contains(secret), "{what} leaked {secret}: {text}");
    }
}

/// Report `input` tokens on a thread the tenant's agent claims for it.
async fn report_usage(h: &Harness, t: &Tenant, thread: maidan_types::ThreadId, input: i64) {
    let claimed = h.store.assign_thread(thread, t.agent).await.unwrap();
    let lease = claimed.claim_lease_id.unwrap();
    let resp = h
        .client
        .post(format!("{}/threads/{}/usage", h.base(), thread.0))
        .bearer_auth(&t.agent_token)
        .json(&json!({
            "usage_report_id": uuid::Uuid::now_v7(),
            "claim_lease_id": lease.0,
            "model": "test/model",
            "tokens": {
                "input": input,
                "output": 0,
                "cache_read": 0,
                "cache_write_5m": 0,
                "cache_write_1h": 0
            },
            "usd_micros": 0,
            "price_snapshot": {
                "input_usd_micros_per_million": 0,
                "output_usd_micros_per_million": 0,
                "cache_read_usd_micros_per_million": 0,
                "cache_write_5m_usd_micros_per_million": 0,
                "cache_write_1h_usd_micros_per_million": 0
            },
            "turns": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "{}",
        resp.text().await.unwrap()
    );
}

#[tokio::test]
async fn usage_rollups_sum_the_callers_workspace_and_refuse_another() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    report_usage(&h, &a, a.thread, 100).await;
    report_usage(&h, &b, b.thread, 7).await;

    for path in [
        format!("/threads/{}/usage-rollup", a.thread.0),
        format!("/workspaces/{}/usage-rollup", a.workspace.0),
        format!("/members/{}/usage-rollup", a.agent.0),
    ] {
        let (s, rollup) = read(&h, &path, &a.admin_token).await;
        assert_eq!(s, StatusCode::OK, "{path}: {rollup}");
        assert_eq!(rollup["workspace_id"], json!(a.workspace.0), "{path}");
        assert_eq!(rollup["reports"], 1, "{path}: {rollup}");
        assert_eq!(rollup["input_tokens"], 100, "B's 7 stay out of {path}");

        let refused = read(&h, &path, &b.admin_token).await;
        assert_refused(&path, &refused, &[a.workspace.0.to_string()]);
    }
    let (_, thread) = read(
        &h,
        &format!("/threads/{}/usage-rollup", a.thread.0),
        &a.admin_token,
    )
    .await;
    assert_eq!(thread["thread_id"], json!(a.thread.0));
    let (_, member) = read(
        &h,
        &format!("/members/{}/usage-rollup", a.agent.0),
        &a.admin_token,
    )
    .await;
    assert_eq!(member["member_id"], json!(a.agent.0));

    // B's own rollup is B's alone, so the refusals above are the boundary.
    let (s, own) = read(
        &h,
        &format!("/workspaces/{}/usage-rollup", b.workspace.0),
        &b.admin_token,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(own["input_tokens"], 7, "{own}");
    h.shutdown().await;
}

#[tokio::test]
async fn deferred_threads_list_the_overlap_and_refuse_another_workspace() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    let lane: maidan_types::Footprint = serde_json::from_value(json!({
        "repo": "https://github.com/Example/App.git",
        "mode": "exclusive",
        "items": [{ "kind": "lane", "value": "bridge" }]
    }))
    .unwrap();
    let lane = lane.validated().unwrap();
    let waiting = new_thread(h.store.as_ref(), a.channel, "waits for the bridge").await;
    h.store.claim_thread(a.thread, a.agent).await.unwrap();
    for thread in [a.thread, waiting] {
        let conflicts = h
            .store
            .set_thread_footprint(thread, Some(lane.clone()))
            .await
            .unwrap();
        assert!(conflicts.is_empty(), "{conflicts:?}");
    }

    let path = format!("/workspaces/{}/deferred", a.workspace.0);
    let (s, deferred) = read(&h, &path, &a.admin_token).await;
    assert_eq!(s, StatusCode::OK, "{deferred}");
    let deferred = deferred.as_array().unwrap();
    assert_eq!(
        deferred.len(),
        1,
        "only the unclaimed overlap: {deferred:?}"
    );
    assert_eq!(deferred[0]["thread_id"], json!(waiting.0));
    assert_eq!(deferred[0]["title"], json!("waits for the bridge"));
    assert!(
        deferred[0]["reasons"].to_string().contains("overlap"),
        "{deferred:?}"
    );

    let refused = read(&h, &path, &b.admin_token).await;
    assert_refused(&path, &refused, &[waiting.0.to_string()]);
    let (s, own) = read(
        &h,
        &format!("/workspaces/{}/deferred", b.workspace.0),
        &b.admin_token,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(own, json!([]), "B holds nothing back");
    h.shutdown().await;
}

#[tokio::test]
async fn member_decisions_are_the_members_own_and_refuse_another_workspace() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    // The admin follows the channel; the agent's result is a decision buried
    // in the admin's feed.
    for t in [&a, &b] {
        h.store.follow_channel(t.admin, t.channel).await.unwrap();
        h.store.claim_thread(t.thread, t.agent).await.unwrap();
        h.store
            .set_thread_result_claiming(t.thread, t.agent, &json!({"status": "done"}))
            .await
            .unwrap();
    }

    let path = format!("/members/{}/decisions", a.admin.0);
    let (s, decisions) = read(&h, &path, &a.admin_token).await;
    assert_eq!(s, StatusCode::OK, "{decisions}");
    let decisions = decisions.as_array().unwrap();
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    assert_eq!(decisions[0]["thread_id"], json!(a.thread.0));
    assert_eq!(decisions[0]["produced_by"], json!(a.agent.0));

    // Personal state: a colleague in the same workspace is refused too.
    let (s, body) = read(&h, &path, &a.agent_token).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let refused = read(&h, &path, &b.admin_token).await;
    assert_eq!(
        refused.0,
        StatusCode::NOT_FOUND,
        "another workspace's member"
    );
    assert_refused(&path, &refused, &[a.thread.0.to_string()]);

    let (s, own) = read(
        &h,
        &format!("/members/{}/decisions", b.admin.0),
        &b.admin_token,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(own[0]["thread_id"], json!(b.thread.0), "{own}");
    assert_eq!(own.as_array().unwrap().len(), 1, "{own}");
    h.shutdown().await;
}

#[tokio::test]
async fn a_push_subscription_is_deleted_only_by_its_own_member() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    let sub = h
        .store
        .add_push_subscription(maidan_types::NewPushSubscription {
            member_id: a.admin,
            endpoint: "https://push.example/a".into(),
            p256dh: "p256dh".into(),
            auth: "auth".into(),
        })
        .await
        .unwrap()
        .id;
    let path = format!("/members/{}/push-subscriptions/{}", a.admin.0, sub.0);
    let listed = || async {
        h.store
            .list_push_subscriptions(a.admin)
            .await
            .unwrap()
            .len()
    };

    let (s, _) = call(&h, reqwest::Method::DELETE, &path, &b.admin_token).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "another workspace's member");
    // B names its own member with A's subscription id: the store scopes the
    // delete to the member, so nothing of A's goes.
    let (s, _) = call(
        &h,
        reqwest::Method::DELETE,
        &format!("/members/{}/push-subscriptions/{}", b.admin.0, sub.0),
        &b.admin_token,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&h, reqwest::Method::DELETE, &path, &a.agent_token).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a colleague's subscription");
    assert_eq!(listed().await, 1, "every refusal left it in place");

    let (s, _) = call(&h, reqwest::Method::DELETE, &path, &a.admin_token).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(listed().await, 0);
    let (s, _) = call(&h, reqwest::Method::DELETE, &path, &a.admin_token).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a second delete finds nothing");
    h.shutdown().await;
}

#[tokio::test]
async fn recent_threads_put_the_last_bumped_first_and_refuse_another_workspace() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    let quiet = new_thread(h.store.as_ref(), a.channel, "quiet").await;
    // Timestamps are compared, so the post has to land measurably later.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    // Posted over REST: the route's post is the one that bumps the thread.
    let posted = h
        .client
        .post(format!("{}/threads/{}/messages", h.base(), a.thread.0))
        .bearer_auth(&a.agent_token)
        .json(&json!({ "body": "bump" }))
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), StatusCode::CREATED);

    let path = format!("/channels/{}/recent-threads", a.channel.0);
    let (s, threads) = read(&h, &path, &a.admin_token).await;
    assert_eq!(s, StatusCode::OK, "{threads}");
    let ids: Vec<&Value> = threads
        .as_array()
        .unwrap()
        .iter()
        .map(|t| &t["id"])
        .collect();
    assert_eq!(ids, vec![&json!(a.thread.0), &json!(quiet.0)], "{threads}");
    let (_, one) = read(&h, &format!("{path}?limit=1"), &a.admin_token).await;
    assert_eq!(one.as_array().unwrap().len(), 1, "{one}");

    let refused = read(&h, &path, &b.admin_token).await;
    assert_refused(
        &path,
        &refused,
        &[a.thread.0.to_string(), quiet.0.to_string()],
    );
    let (s, own) = read(
        &h,
        &format!("/channels/{}/recent-threads", b.channel.0),
        &b.admin_token,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(own[0]["id"], json!(b.thread.0), "{own}");
    h.shutdown().await;
}

#[tokio::test]
async fn a_peer_is_deleted_only_through_its_own_workspace() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    let mut peers = Vec::new();
    for (t, secret) in [(&a, "peer-secret-a"), (&b, "peer-secret-b")] {
        let peer = h
            .store
            .create_peer(maidan_types::NewPeer {
                workspace_id: t.workspace,
                remote_workspace_id: t.workspace,
                name: "remote".into(),
                base_url: "https://remote.example".into(),
                token_hash: hash_secret(secret),
                outbound_secret_ciphertext: None,
            })
            .await
            .unwrap();
        peers.push(peer.id);
    }
    let (peer_a, peer_b) = (peers[0], peers[1]);
    let a_peers = || async { h.store.list_peers(a.workspace).await.unwrap().len() };

    // B names A's workspace: refused before the peer is looked up.
    let path = format!("/workspaces/{}/peers/{}", a.workspace.0, peer_a.0);
    let refused = call(&h, reqwest::Method::DELETE, &path, &b.admin_token).await;
    assert_refused(&path, &refused, &[]);
    // B names its own workspace with A's peer id: the peer is not B's.
    let (s, _) = call(
        &h,
        reqwest::Method::DELETE,
        &format!("/workspaces/{}/peers/{}", b.workspace.0, peer_a.0),
        &b.admin_token,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(a_peers().await, 1, "A's peer survives both");

    let (s, _) = call(&h, reqwest::Method::DELETE, &path, &a.admin_token).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(a_peers().await, 0);
    let (s, _) = call(&h, reqwest::Method::DELETE, &path, &a.admin_token).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "already gone");
    assert_eq!(
        h.store.list_peers(b.workspace).await.unwrap()[0].id,
        peer_b,
        "B's peer is untouched"
    );
    h.shutdown().await;
}

#[tokio::test]
async fn dead_mail_lists_and_requeues_only_the_callers_workspace() {
    let h = spawn().await;
    let a = tenant(h.store.as_ref(), "a").await;
    let b = tenant(h.store.as_ref(), "b").await;
    for (workspace, to) in [
        (Some(a.workspace), "a@example.com"),
        (Some(b.workspace), "b@example.com"),
        (None, "nobody@example.com"),
    ] {
        h.store
            .enqueue_mail(maidan_types::NewMailOutbox {
                workspace_id: workspace,
                source_log_id: None,
                to_address: to.into(),
                subject: format!("for {to}"),
                body: "b".into(),
            })
            .await
            .unwrap()
            .unwrap();
    }
    // Dead-letter all three: a failure with no retry time is final.
    while let Some(mail) = h
        .store
        .claim_next_due_mail(chrono::Utc::now(), 60)
        .await
        .unwrap()
    {
        h.store
            .mark_mail_failed(mail.id, "bounced", None)
            .await
            .unwrap();
    }
    let dead = |token: String| {
        let h = &h;
        async move {
            let (s, rows) = read(h, "/operator/mail/dead", &token).await;
            assert_eq!(s, StatusCode::OK, "{rows}");
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| r["to_address"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(dead(a.admin_token.clone()).await, vec!["a@example.com"]);
    assert_eq!(dead(b.admin_token.clone()).await, vec!["b@example.com"]);
    let global = mint_token(
        h.store.as_ref(),
        a.workspace,
        a.admin,
        vec![
            capability::TOKEN_ADMIN.into(),
            capability::OPERATOR_GLOBAL.into(),
        ],
    )
    .await;
    let mut all = dead(global).await;
    all.sort();
    assert_eq!(
        all,
        vec!["a@example.com", "b@example.com", "nobody@example.com"],
        "operator:global sees every tenant's and the tenant-less row"
    );

    let rows = h.store.list_dead_mail(Some(a.workspace), 10).await.unwrap();
    let a_mail = rows[0].id;
    let requeue = format!("/operator/mail/dead/{}/requeue", a_mail.0);
    let (s, _) = call(&h, reqwest::Method::POST, &requeue, &b.admin_token).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "B cannot re-send A's mail");
    assert_eq!(dead(a.admin_token.clone()).await, vec!["a@example.com"]);

    let (s, _) = call(&h, reqwest::Method::POST, &requeue, &a.admin_token).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(dead(a.admin_token.clone()).await.is_empty(), "requeued");
    let (s, _) = call(&h, reqwest::Method::POST, &requeue, &a.admin_token).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "no longer dead");
    assert_eq!(dead(b.admin_token.clone()).await, vec!["b@example.com"]);
    h.shutdown().await;
}
