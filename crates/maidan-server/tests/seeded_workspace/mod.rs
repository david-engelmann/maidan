//! A workspace seeded with one of every kind the HTTP API names by id, and a
//! server to call it on.
//!
//! The seeding is driven by the served OpenAPI document: every operation that
//! creates a kind some path names by id is called with a body built from its
//! schema, so a new route's kind is seeded without being registered here.
//! `tenant_isolation_e2e` calls every operation with another tenant's token;
//! `http_operation_kinds_e2e` calls them with the owner's own.

// Each test that includes this module uses a different part of it.
#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
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
use sqlx::{
    sqlite::{SqlitePool, SqlitePoolOptions},
    Row,
};
use uuid::Uuid;

pub const SECRET_BODY: &str = "tenant-b-secret-body-7f3a";
pub const SECRET_CHANNEL: &str = "tenant-b-secret-channel";

pub struct Harness {
    pub addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    pub client: reqwest::Client,
    pub store: Arc<dyn Store>,
    /// The database behind `store`, for reading what a call wrote.
    pub pool: SqlitePool,
    _dir: tempfile::TempDir,
}

impl Harness {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub async fn send(
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

pub async fn spawn() -> Harness {
    spawn_with(|_| {}).await
}

/// [`spawn`], with `configure` applied to the state before the router is built.
pub async fn spawn_with(configure: impl FnOnce(&mut AppState)) -> Harness {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let key = Some(Arc::new([7u8; 32]));
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(false, key.clone()),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.webhooks = maidan_server::WebhookRuntime::new(key.clone());
    state.slash = maidan_server::SlashRuntime::new(key.clone());
    state.fsm_hooks = maidan_server::FsmHookRuntime::new(key);
    state.subscribe_resume_secret = Some(Arc::from(
        maidan_server::subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET,
    ));
    configure(&mut state);
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
        pool,
        _dir: dir,
    }
}

pub async fn member(
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

pub async fn token(
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

/// The victim workspace: its owner's token and the ids of what it holds,
/// keyed by the path segment that names each kind (`threads`, `webhooks`).
pub struct Victim {
    pub bearer: String,
    pub ids: BTreeMap<String, String>,
}

impl Victim {
    pub fn id(&self, segment: &str) -> Option<&str> {
        self.ids.get(segment).map(String::as_str)
    }

    /// The victim's id for a field or parameter name in a request, so a body
    /// that names a thread or member names the victim's.
    pub fn id_for_name(&self, name: &str) -> Option<&str> {
        let n = name.to_ascii_lowercase();
        let seg = if n.contains("workspace") {
            "workspaces"
        } else if n.contains("channel") {
            "channels"
        } else if n.contains("thread") || n == "depends_on" || n == "dep_id" || n == "parent_id" {
            "threads"
        } else if n.contains("message") {
            "messages"
        } else if n.contains("member")
            || n.contains("assignee")
            || n.contains("owner")
            || n.contains("reviewer")
            || n.contains("subject")
            || n == "followed_id"
            || n.contains("user")
            || n.contains("delegate")
        {
            if n.contains("delegate")
                || n.contains("reviewer")
                || n == "followed_id"
                || n.contains("other")
            {
                return self.id("reviewers");
            }
            "members"
        } else if n.contains("grant") {
            "delegation-grants"
        } else if n.contains("app") {
            "apps"
        } else {
            return None;
        };
        self.id(seg)
    }
}

/// Values for fields whose schema is a bare string but which the server
/// checks against a closed set.
const FIELD_HINTS: &[(&str, &str)] = &[
    ("capabilities", "workspace:read"),
    ("capability", "workspace:read"),
    ("handler_kind", "http"),
    ("event_kinds", "message_posted"),
    ("events", "message_posted"),
    ("kinds", "message_posted"),
    ("kind", "attachment"),
    ("action", "start_review"),
    ("q", "tenant"),
    ("handler_target", "https://example.com/tenant-b"),
    ("selector", "C0TENANTB"),
    ("platform", "slack"),
];

/// Segments whose parameter is a string key rather than an id, and the
/// request field that carries it when the thing is created.
const KEYED_BY_FIELD: &[(&str, &str)] = &[
    ("secrets", "name"),
    ("glossary", "term"),
    ("skills", "skill"),
    ("required-skills", "skill"),
    ("slack-links", "slack_channel_id"),
];

fn resolve<'a>(doc: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => {
            let name = r.rsplit('/').next().unwrap();
            resolve(doc, &doc["components"]["schemas"][name])
        }
        None => schema,
    }
}

/// A value satisfying `schema`, with every id-like field pointing into the
/// victim workspace. Only required properties are filled.
pub fn example(doc: &Value, schema: &Value, name: &str, victim: &Victim, depth: usize) -> Value {
    let schema = resolve(doc, schema);
    if depth > 8 {
        return Value::Null;
    }
    if let Some(v) = schema.get("const") {
        return v.clone();
    }
    if let Some(Value::Array(values)) = schema.get("enum") {
        if let Some(v) = values.iter().find(|v| !v.is_null()) {
            return v.clone();
        }
    }
    if let Some(Value::Array(all)) = schema.get("allOf") {
        let mut merged = serde_json::Map::new();
        for part in all {
            if let Value::Object(m) = example(doc, part, name, victim, depth + 1) {
                merged.extend(m);
            }
        }
        return Value::Object(merged);
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(Value::Array(options)) = schema.get(key) {
            let pick = options
                .iter()
                .find(|o| resolve(doc, o).get("type") != Some(&json!("null")))
                .unwrap_or(&options[0]);
            return example(doc, pick, name, victim, depth + 1);
        }
    }
    if let Some((_, hint)) = FIELD_HINTS.iter().find(|(n, _)| *n == name) {
        let is_array = schema.get("type") == Some(&json!("array"))
            || schema["type"]
                .as_array()
                .is_some_and(|t| t.contains(&json!("array")));
        return if is_array { json!([hint]) } else { json!(hint) };
    }
    let ty = match schema.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("null"),
        _ if schema.get("properties").is_some() => "object",
        _ => "string",
    };
    match ty {
        "object" => {
            let mut out = serde_json::Map::new();
            let required: Vec<&str> = schema["required"]
                .as_array()
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if let Some(props) = schema["properties"].as_object() {
                for (key, prop) in props {
                    if required.contains(&key.as_str()) {
                        out.insert(key.clone(), example(doc, prop, key, victim, depth + 1));
                    }
                }
            }
            Value::Object(out)
        }
        "array" => {
            let min = schema["minItems"].as_u64().unwrap_or(0).max(1);
            let item = example(doc, &schema["items"], name, victim, depth + 1);
            Value::Array((0..min).map(|_| item.clone()).collect())
        }
        "integer" => json!(schema["minimum"].as_i64().unwrap_or(1).max(1)),
        "number" => json!(schema["minimum"].as_f64().unwrap_or(1.0).max(1.0)),
        "boolean" => json!(false),
        "null" => Value::Null,
        _ => {
            let format = schema["format"].as_str().unwrap_or("");
            match format {
                "uuid" => json!(victim
                    .id_for_name(name)
                    .map(str::to_string)
                    .unwrap_or_else(|| Uuid::now_v7().to_string())),
                "date-time" => {
                    json!((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339())
                }
                "uri" | "url" => json!("https://example.com/tenant-b"),
                "email" => json!("tenant-b@example.com"),
                _ => {
                    let n = name.to_ascii_lowercase();
                    if n.contains("url") || n.contains("endpoint") {
                        json!("https://example.com/tenant-b")
                    } else if n.contains("email") {
                        json!("tenant-b@example.com")
                    } else if n == "sha" || n.ends_with("_sha") || n.contains("sha256") {
                        json!("0".repeat(64))
                    } else if n.ends_with("_id") {
                        json!(victim.id_for_name(&n).map(str::to_string).unwrap_or_else(
                            || format!("tb{}", &Uuid::now_v7().simple().to_string()[..10])
                        ))
                    } else {
                        let min = schema["minLength"].as_u64().unwrap_or(1) as usize;
                        json!(format!("tb{}", &Uuid::now_v7().simple().to_string()[20..])
                            .chars()
                            .cycle()
                            .take(min.max(12))
                            .collect::<String>())
                    }
                }
            }
        }
    }
}

pub fn json_body_schema(op: &Value) -> Option<&Value> {
    op["requestBody"]["content"]["application/json"]["schema"]
        .as_object()
        .map(|_| &op["requestBody"]["content"]["application/json"]["schema"])
}

/// The operation's required query parameters, filled like body fields.
pub fn query(doc: &Value, op: &Value, victim: &Victim) -> String {
    let mut pairs = Vec::new();
    for p in op["parameters"].as_array().into_iter().flatten() {
        let p = resolve(doc, p);
        if p["in"] == "query" && p["required"] == true {
            let name = p["name"].as_str().unwrap_or_default();
            let v = example(doc, &p["schema"], name, victim, 0);
            let v = match v {
                Value::String(s) => s,
                Value::Array(a) => a
                    .first()
                    .map(|x| x.as_str().map(str::to_string).unwrap_or(x.to_string()))
                    .unwrap_or_default(),
                other => other.to_string(),
            };
            pairs.push(format!("{name}={v}"));
        }
    }
    if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.join("&"))
    }
}

/// Fill a path template with the victim's ids. `None` if a parameter names a
/// kind the victim does not hold.
pub fn fill(template: &str, victim: &Victim) -> Option<String> {
    let segments: Vec<&str> = template.split('/').collect();
    let mut out = Vec::with_capacity(segments.len());
    for (i, seg) in segments.iter().enumerate() {
        if seg.starts_with('{') {
            let prev = if i > 0 { segments[i - 1] } else { "" };
            // `/threads/{id}/deliveries/{did}` is a result delivery, not a
            // webhook delivery: a kind can be keyed by its parent too.
            let parent = if i > 2 { segments[i - 3] } else { "" };
            let id = victim
                .id(&format!("{parent}/{prev}"))
                .or_else(|| victim.id(prev))?;
            out.push(id.to_string());
        } else {
            out.push((*seg).to_string());
        }
    }
    Some(out.join("/"))
}

/// Seed the victim: a private channel, a thread, a message and a second
/// member by hand, then every other kind a path names, by calling the
/// operation that creates it with a body built from its schema. Repeats while
/// it makes progress, since some kinds live under others.
pub async fn seed_victim(h: &Harness, doc: &Value) -> Victim {
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
    let mut victim = Victim {
        bearer,
        ids: BTreeMap::new(),
    };
    victim.ids.insert("workspaces".into(), ws.0.to_string());
    // `members` is the owner, so self-scoped kinds (skills, push
    // subscriptions, notifications) can be created under it.
    victim.ids.insert("members".into(), owner.0.to_string());
    for alias in ["member-follows", "reviewers"] {
        victim.ids.insert(alias.into(), other.0.to_string());
    }
    let third = member(h.store.as_ref(), ws, "b-third", MemberKind::Human).await;

    let channel = h
        .send(
            Method::POST,
            &format!("/workspaces/{}/channels", ws.0),
            &victim.bearer,
            Some(json!({ "name": SECRET_CHANNEL, "private": true })),
        )
        .await;
    let channel = created_id(channel).await;
    let thread = h
        .send(
            Method::POST,
            &format!("/channels/{channel}/threads"),
            &victim.bearer,
            Some(json!({})),
        )
        .await;
    let thread = created_id(thread).await;
    let message = h
        .send(
            Method::POST,
            &format!("/threads/{thread}/messages"),
            &victim.bearer,
            Some(json!({ "body": SECRET_BODY })),
        )
        .await;
    let message = created_id(message).await;
    for seg in ["channels", "channel-follows"] {
        victim.ids.insert(seg.into(), channel.clone());
    }
    for seg in ["threads", "thread-follows", "dependencies"] {
        victim.ids.insert(seg.into(), thread.clone());
    }
    victim.ids.insert("messages".into(), message);
    // A second thread for dependency edges to point at.
    let dep = h
        .send(
            Method::POST,
            &format!("/channels/{channel}/threads"),
            &victim.bearer,
            Some(json!({})),
        )
        .await;
    let dep = created_id(dep).await;
    let _ = h
        .send(
            Method::POST,
            &format!("/threads/{thread}/dependencies"),
            &victim.bearer,
            Some(json!({ "depends_on": dep })),
        )
        .await;

    seed_by_hand(h, &mut victim, &[owner, other, third]).await;

    loop {
        let before = victim.ids.len();
        for (template, item) in doc["paths"].as_object().unwrap() {
            let Some(op) = item.get("post").or_else(|| item.get("put")) else {
                continue;
            };
            let method = if item.get("post").is_some() {
                Method::POST
            } else {
                Method::PUT
            };
            let segment = template.rsplit('/').next().unwrap();
            if segment.starts_with('{') || victim.ids.contains_key(segment) {
                continue;
            }
            // Only kinds some path names by id.
            let named = doc["paths"]
                .as_object()
                .unwrap()
                .keys()
                .any(|p| p.contains(&format!("/{segment}/{{")));
            if !named {
                continue;
            }
            let Some(path) = fill(template, &victim) else {
                continue;
            };
            let body = json_body_schema(op)
                .map(|s| example(doc, s, "", &victim, 0))
                .unwrap_or_else(|| json!({}));
            let path = format!("{path}{}", query(doc, op, &victim));
            let res = h
                .send(method, &path, &victim.bearer, Some(body.clone()))
                .await;
            if !res.status().is_success() {
                if std::env::var("TENANT_DEBUG").is_ok() {
                    let st = res.status();
                    println!(
                        "seed {template}: {st} {}",
                        res.text()
                            .await
                            .unwrap_or_default()
                            .chars()
                            .take(200)
                            .collect::<String>()
                    );
                }
                continue;
            }
            let created: Value = res.json().await.unwrap_or(Value::Null);
            let key = KEYED_BY_FIELD
                .iter()
                .find(|(s, _)| *s == segment)
                .and_then(|(_, field)| {
                    body[*field]
                        .as_str()
                        .or_else(|| created[*field].as_str())
                        .map(str::to_string)
                });
            let pick = |v: &Value| {
                ["id", "job_id", "sha256", "sha", "upload_id", "name"]
                    .iter()
                    .find_map(|k| {
                        v[*k]
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| v[*k].as_i64().map(|n| n.to_string()))
                    })
            };
            // Some creates wrap the thing: `{"webhook": {...}, "secret": ...}`.
            let nested = || {
                created
                    .as_object()
                    .and_then(|o| o.values().filter(|v| v.is_object()).find_map(pick))
            };
            let id = key.or_else(|| pick(&created)).or_else(nested);
            if std::env::var("TENANT_DEBUG").is_ok() {
                println!(
                    "seeded {template} -> {id:?} from {}",
                    created.to_string().chars().take(160).collect::<String>()
                );
            }
            if let Some(id) = id {
                victim.ids.insert(segment.to_string(), id);
            }
        }
        if victim.ids.len() == before {
            break;
        }
    }
    seed_server_made(h, &mut victim, owner).await;
    victim
}

/// Kinds the server makes as a side effect (an install, a gate, a
/// notification, a delivery), written directly.
async fn seed_server_made(h: &Harness, victim: &mut Victim, owner: MemberId) {
    let ws = victim.ids["workspaces"].clone();
    let ws_id = WorkspaceId(ws.parse().unwrap());
    if let Some(app) = victim.ids.get("apps").cloned() {
        let res = h
            .send(
                Method::POST,
                &format!("/workspaces/{ws}/apps/{app}/install"),
                &victim.bearer,
                Some(json!({ "granted_capabilities": ["workspace:read"] })),
            )
            .await;
        let body: Value = res.json().await.unwrap_or(Value::Null);
        let id = body["id"]
            .as_str()
            .or(body["installation"]["id"].as_str())
            .map(str::to_string);
        if let Some(id) = id {
            victim.ids.insert("app-installations".into(), id);
        }
    }
    let thread = maidan_types::ThreadId(victim.ids["threads"].parse().unwrap());
    let gate = h
        .store
        .create_approval_gate(&maidan_types::NewApprovalGate {
            workspace_id: ws_id,
            thread_id: Some(thread),
            requested_by: owner,
            prompt: SECRET_BODY.into(),
            schema: None,
        })
        .await
        .unwrap();
    victim
        .ids
        .insert("approval-gates".into(), gate.id.0.to_string());
    let note = h
        .store
        .create_notification(maidan_types::NewNotification {
            workspace_id: ws_id,
            member_id: owner,
            kind: maidan_types::EventKind::MessagePosted,
            source_log_id: 1,
            channel_id: None,
            thread_id: Some(thread),
            message_id: None,
            actor_id: None,
        })
        .await
        .unwrap();
    victim
        .ids
        .insert("notifications".into(), note.id.0.to_string());
    if let Some(delivery) = h
        .store
        .arm_unroutable_result_delivery(thread, "slack", "C0TENANTB", chrono::Utc::now())
        .await
        .unwrap()
    {
        victim
            .ids
            .insert("threads/deliveries".into(), delivery.id.0.to_string());
    }
    if let Some(hook) = victim.ids.get("webhooks") {
        let delivery = h
            .store
            .enqueue_webhook_delivery(
                maidan_types::WebhookSubscriptionId(hook.parse().unwrap()),
                1,
                "{}",
            )
            .await
            .unwrap();
        victim.ids.insert("deliveries".into(), delivery.to_string());
    }
}

/// Kinds the schema-driven pass cannot create: a raw upload, a group of
/// three, a key in the path, and things only the server makes (a
/// notification, an approval gate, a delivery).
async fn seed_by_hand(h: &Harness, victim: &mut Victim, members: &[MemberId]) {
    let ws = victim.ids["workspaces"].clone();
    let thread = victim.ids["threads"].clone();
    let bearer = victim.bearer.clone();
    let res = h
        .client
        .post(h.url("/artifacts?kind=attachment"))
        .header("Authorization", format!("Bearer {bearer}"))
        .header("Content-Type", "application/octet-stream")
        .body(format!("{SECRET_BODY} artifact"))
        .send()
        .await
        .unwrap();
    if let Some(sha) = res.json::<Value>().await.ok().and_then(|v| {
        v["sha256"]
            .as_str()
            .or(v["sha"].as_str())
            .map(str::to_string)
    }) {
        victim.ids.insert("artifacts".into(), sha);
    }
    let ids: Vec<String> = members.iter().map(|m| m.0.to_string()).collect();
    let res = h
        .send(
            Method::POST,
            &format!("/workspaces/{ws}/group-dms"),
            &bearer,
            Some(json!({ "member_ids": ids })),
        )
        .await;
    if let Some(id) = res
        .json::<Value>()
        .await
        .ok()
        .and_then(|v| v["id"].as_str().map(str::to_string))
    {
        victim.ids.insert("group-dms".into(), id);
    }
    let res = h
        .send(
            Method::PUT,
            &format!("/workspaces/{ws}/glossary/tenantbterm"),
            &bearer,
            Some(json!({ "definition": "tenant b" })),
        )
        .await;
    if res.status().is_success() {
        victim.ids.insert("glossary".into(), "tenantbterm".into());
    }
    let res = h
        .send(
            Method::POST,
            &format!("/workspaces/{ws}/dm"),
            &bearer,
            Some(json!({ "other_member_id": members[1].0 })),
        )
        .await;
    if let Some(id) = res
        .json::<Value>()
        .await
        .ok()
        .and_then(|v| v["id"].as_str().map(str::to_string))
    {
        victim.ids.insert("dm".into(), id);
    }
    // A mention of the owner puts a notification in the owner's inbox.
    let owner = members[0].0.to_string();
    let _ = h
        .send(
            Method::POST,
            &format!("/threads/{thread}/messages"),
            &bearer,
            Some(json!({ "body": "@b-owner look", "mentions": [owner] })),
        )
        .await;
    let _ = thread;
}

/// Every row of every table, as text, per table.
pub async fn snapshot(pool: &sqlx::SqlitePool) -> BTreeMap<String, Vec<String>> {
    let tables: Vec<String> = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' AND name != 'maidan_audit'",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.get::<String, _>("name"))
    .collect();
    let mut out = BTreeMap::new();
    for table in tables {
        let columns: Vec<String> = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.get::<String, _>("name"))
            .collect();
        let row = columns
            .iter()
            .map(|c| format!("quote(\"{c}\")"))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let mut rows: Vec<String> = sqlx::query(&format!("SELECT {row} AS r FROM \"{table}\""))
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.get::<String, _>("r"))
            .collect();
        rows.sort();
        out.insert(table, rows);
    }
    out
}

/// The tables `after` differs from `before` in, with a changed row of each.
pub fn changed_tables(
    before: &BTreeMap<String, Vec<String>>,
    after: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    after
        .iter()
        .filter_map(|(table, rows)| {
            let old = before.get(table).cloned().unwrap_or_default();
            (rows != &old).then(|| {
                let old: BTreeSet<_> = old.iter().collect();
                let new: BTreeSet<_> = rows.iter().collect();
                let sample = new
                    .symmetric_difference(&old)
                    .next()
                    .map(|r| r.chars().take(120).collect::<String>())
                    .unwrap_or_default();
                format!("{table} ({sample})")
            })
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
