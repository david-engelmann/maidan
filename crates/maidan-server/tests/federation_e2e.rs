//! Federation ingress, peer admin, and peer bearer event tail (SQLite).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

fn federation_test_key() -> Option<Arc<[u8; 32]>> {
    Some(Arc::new([0x42; 32]))
}

use maidan_a2a::{FederatedEventBatch, FederationEnvelope};
use maidan_auth::{
    capability::{FEDERATION_ADMIN, WORKSPACE_READ},
    hash_secret, TokenSecret,
};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{
    Event, EventKind, MemberKind, NewApiToken, NewMember, NewWorkspace, PeerId, StoredEvent,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn_with_auth_disabled() -> Harness {
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
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        FederationRuntime::new(true, federation_test_key()),
        Arc::new(AtomicI64::new(0)),
        None,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        store,
        _dir: dir,
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
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, federation_test_key()),
        Arc::new(AtomicI64::new(0)),
        None,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        store,
        _dir: dir,
    }
}

async fn mint_admin_token(store: &dyn Store, workspace_id: WorkspaceId) -> String {
    let member = store
        .create_member(NewMember {
            workspace_id,
            handle: "admin".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id: member.id,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![FEDERATION_ADMIN.into(), WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn federation_ingest_dedupes_and_peer_lists_events() {
    let h = spawn().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "fed".to_string(),
        })
        .await
        .unwrap();
    let admin = mint_admin_token(h.store.as_ref(), ws.id).await;

    let create = h
        .client
        .post(format!("{}/workspaces/{}/peers", h.base(), ws.id.0))
        .bearer_auth(&admin)
        .json(&json!({
            "name": "remote-a",
            "base_url": "https://remote.example"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body: serde_json::Value = create.json().await.unwrap();
    let peer_id = PeerId(uuid::Uuid::parse_str(body["peer"]["id"].as_str().unwrap()).unwrap());
    let peer_secret = body["secret"].as_str().unwrap().to_string();

    let member = h
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let event = Event::MemberJoined {
        occurred_at: chrono::Utc::now(),
        workspace_id: ws.id,
        member: member.clone(),
    };
    let payload = serde_json::to_value(&event).unwrap();
    let stored = StoredEvent {
        id: 1,
        kind: EventKind::MemberJoined,
        workspace_id: Some(ws.id),
        channel_id: None,
        thread_id: None,
        payload,
        occurred_at: chrono::Utc::now(),
    };
    let batch = FederatedEventBatch {
        events: vec![FederationEnvelope {
            origin_peer_id: peer_id,
            remote_event_id: 1,
            event: stored.clone(),
        }],
    };

    let ingest = h
        .client
        .post(format!("{}/a2a/v1/events", h.base()))
        .bearer_auth(&peer_secret)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(ingest.status(), StatusCode::OK);
    let summary: serde_json::Value = ingest.json().await.unwrap();
    assert_eq!(summary["ingested"], 1);
    assert_eq!(summary["skipped"], 0);

    let dup = h
        .client
        .post(format!("{}/a2a/v1/events", h.base()))
        .bearer_auth(&peer_secret)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), StatusCode::OK);
    let summary2: serde_json::Value = dup.json().await.unwrap();
    assert_eq!(summary2["ingested"], 0);
    assert_eq!(summary2["skipped"], 1);

    let tail = h
        .client
        .get(format!(
            "{}/workspaces/{}/events?after_id=0&limit=10",
            h.base(),
            ws.id.0
        ))
        .bearer_auth(&peer_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(tail.status(), StatusCode::OK);
    let events: Vec<StoredEvent> = tail.json().await.unwrap();
    assert!(!events.is_empty());

    let card = h
        .client
        .get(format!("{}/.well-known/maidan.json", h.base()))
        .send()
        .await
        .unwrap();
    assert_eq!(card.status(), StatusCode::OK);

    h.shutdown().await;
}

#[tokio::test]
async fn federation_ingest_accepts_peer_bearer_when_auth_disabled_globally() {
    let h = spawn_with_auth_disabled().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "fed-dev".to_string(),
        })
        .await
        .unwrap();

    let create = h
        .client
        .post(format!("{}/workspaces/{}/peers", h.base(), ws.id.0))
        .json(&json!({
            "name": "remote-a",
            "base_url": "https://remote.example"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body: serde_json::Value = create.json().await.unwrap();
    let peer_id = PeerId(uuid::Uuid::parse_str(body["peer"]["id"].as_str().unwrap()).unwrap());
    let peer_secret = body["secret"].as_str().unwrap().to_string();

    let member = h
        .store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let event = Event::MemberJoined {
        occurred_at: chrono::Utc::now(),
        workspace_id: ws.id,
        member: member.clone(),
    };
    let payload = serde_json::to_value(&event).unwrap();
    let stored = StoredEvent {
        id: 1,
        kind: EventKind::MemberJoined,
        workspace_id: Some(ws.id),
        channel_id: None,
        thread_id: None,
        payload,
        occurred_at: chrono::Utc::now(),
    };
    let batch = FederatedEventBatch {
        events: vec![FederationEnvelope {
            origin_peer_id: peer_id,
            remote_event_id: 1,
            event: stored,
        }],
    };

    let ingest = h
        .client
        .post(format!("{}/a2a/v1/events", h.base()))
        .bearer_auth(&peer_secret)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(ingest.status(), StatusCode::OK);
    h.shutdown().await;
}

#[tokio::test]
async fn federation_ingest_rejects_wrong_peer_bearer() {
    let h = spawn().await;
    let _ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "fed2".to_string(),
        })
        .await
        .unwrap();
    let resp = h
        .client
        .post(format!("{}/a2a/v1/events", h.base()))
        .bearer_auth("not-a-valid-peer-secret")
        .json(&FederatedEventBatch { events: vec![] })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    h.shutdown().await;
}

#[tokio::test]
async fn federation_peer_outbound_secret_hydrates_after_restart() {
    use maidan_auth::{encrypt_peer_secret, hash_secret};
    use maidan_server::federation::{
        forget_peer_secret, hydrate_federation_secrets, resolve_outbound_secret,
    };

    let key = Arc::new([0x42; 32]);
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
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        FederationRuntime::new(true, Some(key.clone())),
        Arc::new(AtomicI64::new(0)),
        None,
    );

    let ws = store
        .create_workspace(NewWorkspace {
            name: "persist".to_string(),
        })
        .await
        .unwrap();
    let plaintext = "maid_peer_secret_for_poll";
    let peer = store
        .create_peer(maidan_types::NewPeer {
            workspace_id: ws.id,
            remote_workspace_id: ws.id,
            name: "remote".to_string(),
            base_url: "https://remote.example".to_string(),
            token_hash: hash_secret(plaintext),
            outbound_secret_ciphertext: Some(
                encrypt_peer_secret(plaintext, key.as_ref()).expect("encrypt"),
            ),
        })
        .await
        .unwrap();
    assert!(peer.outbound_secret_ciphertext.is_some());

    let fresh = AppState::new(
        store,
        state.artifacts.clone(),
        state.bus.clone(),
        state.search.clone(),
        state.embedding_provider.clone(),
        true,
        FederationRuntime::new(true, Some(key)),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    assert!(fresh.federation.outbound_secrets.read().unwrap().is_empty());
    assert_eq!(
        resolve_outbound_secret(&fresh, &peer).as_deref(),
        Some(plaintext)
    );
    assert!(fresh
        .federation
        .outbound_secrets
        .read()
        .unwrap()
        .contains_key(&peer.id));

    forget_peer_secret(&fresh.federation.outbound_secrets, peer.id);
    hydrate_federation_secrets(&fresh).await.expect("hydrate");
    assert_eq!(
        resolve_outbound_secret(&fresh, &peer).as_deref(),
        Some(plaintext)
    );
}
