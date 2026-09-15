//! Cluster 395: named capability sets, holder-side attenuation, room URIs,
//! well-known discovery, and handle aliases that cannot break stored ids.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret, AGENT_WORKER, HUMAN_ADMIN};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId, ROOM_DISCOVERY_TYPE,
    ROOM_TYPE,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn named_sets_room_uris_handles_and_attenuation() {
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
    let bus = Arc::new(InMemoryBus::with_capacity(16));

    let ws = store
        .create_workspace(NewWorkspace {
            name: "room".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let admin_caps = maidan_auth::expand_set(HUMAN_ADMIN).unwrap();
    let admin = mint(store.as_ref(), ws.id, member.id, admin_caps.clone()).await;
    let worker_only = mint(
        store.as_ref(),
        ws.id,
        member.id,
        maidan_auth::expand_set(AGENT_WORKER).unwrap(),
    )
    .await;

    let state = AppState::new(
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
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let admin_b = format!("Bearer {admin}");
    let worker_b = format!("Bearer {worker_only}");
    let wid = ws.id.0;

    // Public well-known: scheme only, no tenant list.
    let disc: Value = client
        .get(format!("{base}/.well-known/maidan-room"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(disc["$type"], ROOM_DISCOVERY_TYPE);
    assert_eq!(disc["id_kind"], "uuid");
    assert_eq!(disc["handle_is_alias"], true);
    assert!(disc.get("rooms").is_none());
    assert!(disc.get("workspace_id").is_none());

    let sets: Value = client
        .get(format!("{base}/capability-sets"))
        .header("Authorization", &admin_b)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sets[0]["name"], AGENT_WORKER);
    assert_eq!(sets[1]["name"], HUMAN_ADMIN);

    let me: Value = client
        .get(format!("{base}/me"))
        .header("Authorization", &admin_b)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let held: Vec<String> = me["capability_sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        held,
        vec![AGENT_WORKER.to_string(), HUMAN_ADMIN.to_string()]
    );

    // Issuer mint: named set + further restrict.
    let minted = client
        .post(format!(
            "{base}/workspaces/{wid}/members/{}/tokens",
            member.id.0
        ))
        .header("Authorization", &admin_b)
        .json(&json!({
            "capability_set": AGENT_WORKER,
            "capabilities": [capability::WORKSPACE_READ, capability::SEARCH_QUERY],
            "label": "restricted-worker"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(minted.status(), StatusCode::CREATED);
    let minted: Value = minted.json().await.unwrap();
    assert_eq!(
        minted["capabilities"],
        json!([capability::WORKSPACE_READ, capability::SEARCH_QUERY])
    );

    let widen = client
        .post(format!(
            "{base}/workspaces/{wid}/members/{}/tokens",
            member.id.0
        ))
        .header("Authorization", &admin_b)
        .json(&json!({
            "capability_set": AGENT_WORKER,
            "capabilities": [capability::TOKEN_ADMIN]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(widen.status(), StatusCode::BAD_REQUEST);

    // Holder attenuation: worker token cannot mint (no token:admin) but can drop rights.
    let mint_denied = client
        .post(format!(
            "{base}/workspaces/{wid}/members/{}/tokens",
            member.id.0
        ))
        .header("Authorization", &worker_b)
        .json(&json!({ "capability_set": AGENT_WORKER }))
        .send()
        .await
        .unwrap();
    assert_eq!(mint_denied.status(), StatusCode::FORBIDDEN);

    let attenuated = client
        .post(format!("{base}/tokens/attenuate"))
        .header("Authorization", &worker_b)
        .json(&json!({
            "capabilities": [capability::WORKSPACE_READ, capability::MESSAGE_POST],
            "label": "weaker"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(attenuated.status(), StatusCode::CREATED);
    let attenuated: Value = attenuated.json().await.unwrap();
    assert_eq!(
        attenuated["capabilities"],
        json!([capability::WORKSPACE_READ, capability::MESSAGE_POST])
    );
    assert!(attenuated["secret"].as_str().unwrap().starts_with("maid_"));

    let amp = client
        .post(format!("{base}/tokens/attenuate"))
        .header("Authorization", &worker_b)
        .json(&json!({ "capabilities": [capability::TOKEN_ADMIN] }))
        .send()
        .await
        .unwrap();
    assert_eq!(amp.status(), StatusCode::BAD_REQUEST);

    // Handle rename does not change the room URI.
    let missing = client
        .get(format!("{base}/workspaces/{wid}/handle"))
        .header("Authorization", &admin_b)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let set = client
        .put(format!("{base}/workspaces/{wid}/handle"))
        .header("Authorization", &admin_b)
        .json(&json!({ "handle": "acme" }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);

    let card: Value = client
        .get(format!("{base}/workspaces/{wid}/room"))
        .header("Authorization", &admin_b)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card["$type"], ROOM_TYPE);
    assert_eq!(card["handle"], "acme");
    assert_eq!(card["uri"], format!("maidan://{wid}"));

    let renamed = client
        .put(format!("{base}/workspaces/{wid}/handle"))
        .header("Authorization", &admin_b)
        .json(&json!({ "handle": "renamed" }))
        .send()
        .await
        .unwrap();
    assert_eq!(renamed.status(), StatusCode::OK);
    let card2: Value = client
        .get(format!("{base}/workspaces/{wid}/room"))
        .header("Authorization", &admin_b)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card2["handle"], "renamed");
    assert_eq!(card2["uri"], card["uri"]);

    let bad = client
        .put(format!("{base}/workspaces/{wid}/handle"))
        .header("Authorization", &admin_b)
        .json(&json!({ "handle": "Acme" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}
