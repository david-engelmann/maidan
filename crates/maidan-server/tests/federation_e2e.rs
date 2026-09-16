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
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Artifact, ArtifactId, ArtifactKind, Event, EventKind, MemberId, MemberKind, NewApiToken,
    NewMember, NewWorkspace, PeerId, StoredEvent, WorkspaceId,
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
            app_installation_id: None,
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
    let link = maidan_types::link_for(1, &payload, None).unwrap();
    let stored = StoredEvent {
        id: 1,
        lsn: 1,
        kind: EventKind::MemberJoined,
        workspace_id: Some(ws.id),
        channel_id: None,
        thread_id: None,
        payload,
        occurred_at: chrono::Utc::now(),
        prev_hash: link.prev_hash,
        content_hash: link.content_hash,
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

/// Cluster 215: the federation ingest allowlist refuses a non-federatable event
/// kind. `ArtifactUpserted` is not federatable (blob bytes aren't transferred),
/// so it is never ingested.
///
/// Cluster 397.6 changed the *reporting*, not the rule: it is counted as
/// `refused` in the summary rather than `403`ing the batch. A peer's chain
/// necessarily contains kinds we do not accept, so refusing the whole batch made
/// a mixed batch unreplicable. The event is still not ingested.
#[tokio::test]
async fn federation_ingest_rejects_non_federatable_artifact_event() {
    let h = spawn().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "fed-art".to_string(),
        })
        .await
        .unwrap();
    let admin = mint_admin_token(h.store.as_ref(), ws.id).await;

    let create = h
        .client
        .post(format!("{}/workspaces/{}/peers", h.base(), ws.id.0))
        .bearer_auth(&admin)
        .json(&json!({ "name": "remote-art", "base_url": "https://remote.example" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body: serde_json::Value = create.json().await.unwrap();
    let peer_id = PeerId(uuid::Uuid::parse_str(body["peer"]["id"].as_str().unwrap()).unwrap());
    let peer_secret = body["secret"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let event = Event::ArtifactUpserted {
        occurred_at: now,
        artifact: Artifact {
            id: ArtifactId(uuid::Uuid::from_u128(9)),
            sha256: "a".repeat(64),
            size_bytes: 3,
            mime_type: Some("text/plain".to_string()),
            kind: ArtifactKind::Attachment,
            uploaded_by: Some(MemberId(uuid::Uuid::from_u128(10))),
            created_at: now,
            tombstoned_at: None,
        },
    };
    let stored = StoredEvent {
        id: 1,
        lsn: 1,
        kind: EventKind::ArtifactUpserted,
        workspace_id: None,
        channel_id: None,
        thread_id: None,
        payload: serde_json::to_value(&event).unwrap(),
        occurred_at: now,
        prev_hash: maidan_types::genesis_hash(),
        content_hash: maidan_types::content_hash(&serde_json::to_value(&event).unwrap()).unwrap(),
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
    let summary: serde_json::Value = ingest.json().await.unwrap();
    assert_eq!(summary["refused"], 1, "refused, not ingested: {summary}");
    assert_eq!(summary["ingested"], 0);

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
    let link = maidan_types::link_for(1, &payload, None).unwrap();
    let stored = StoredEvent {
        id: 1,
        lsn: 1,
        kind: EventKind::MemberJoined,
        workspace_id: Some(ws.id),
        channel_id: None,
        thread_id: None,
        payload,
        occurred_at: chrono::Utc::now(),
        prev_hash: link.prev_hash,
        content_hash: link.content_hash,
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
        false,
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
        false,
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

async fn mint_peer(h: &Harness, ws: WorkspaceId, name: &str) -> (PeerId, String) {
    let admin = mint_admin_token(h.store.as_ref(), ws).await;
    let create = h
        .client
        .post(format!("{}/workspaces/{}/peers", h.base(), ws.0))
        .bearer_auth(&admin)
        .json(&json!({ "name": name, "base_url": "https://remote.example" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body: serde_json::Value = create.json().await.unwrap();
    let peer_id = PeerId(uuid::Uuid::parse_str(body["peer"]["id"].as_str().unwrap()).unwrap());
    let peer_secret = body["secret"].as_str().unwrap().to_string();
    (peer_id, peer_secret)
}

fn member_joined_envelope(
    peer_id: PeerId,
    id: i64,
    event: &Event,
    previous: Option<&maidan_types::EventLink>,
) -> (FederationEnvelope, maidan_types::EventLink) {
    let payload = serde_json::to_value(event).unwrap();
    let link = maidan_types::link_for(id, &payload, previous).unwrap();
    let stored = StoredEvent {
        id,
        lsn: id,
        kind: EventKind::MemberJoined,
        workspace_id: event.workspace_id(),
        channel_id: None,
        thread_id: None,
        payload,
        occurred_at: chrono::Utc::now(),
        prev_hash: link.prev_hash.clone(),
        content_hash: link.content_hash.clone(),
    };
    (
        FederationEnvelope {
            origin_peer_id: peer_id,
            remote_event_id: id,
            event: stored,
        },
        link,
    )
}

#[tokio::test]
async fn federation_ingest_rejects_payload_rewrite() {
    let h = spawn().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "fed-hash".to_string(),
        })
        .await
        .unwrap();
    let (peer_id, peer_secret) = mint_peer(&h, ws.id, "remote-hash").await;
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
        member,
    };
    let (mut envelope, _) = member_joined_envelope(peer_id, 1, &event, None);
    envelope.event.payload["kind"] = json!("message_posted");
    let ingest = h
        .client
        .post(format!("{}/a2a/v1/events", h.base()))
        .bearer_auth(&peer_secret)
        .json(&FederatedEventBatch {
            events: vec![envelope],
        })
        .send()
        .await
        .unwrap();
    assert_eq!(ingest.status(), StatusCode::CONFLICT);
    let problem: serde_json::Value = ingest.json().await.unwrap();
    assert_eq!(
        problem["type"],
        "https://maidan.dev/problems/event-log-broken"
    );
    h.shutdown().await;
}

#[tokio::test]
async fn federation_ingest_rejects_prev_hash_break() {
    let h = spawn().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "fed-prev".to_string(),
        })
        .await
        .unwrap();
    let (peer_id, peer_secret) = mint_peer(&h, ws.id, "remote-prev").await;
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
    let first = Event::MemberJoined {
        occurred_at: chrono::Utc::now(),
        workspace_id: ws.id,
        member: member.clone(),
    };
    let (envelope1, link1) = member_joined_envelope(peer_id, 1, &first, None);
    let ok = h
        .client
        .post(format!("{}/a2a/v1/events", h.base()))
        .bearer_auth(&peer_secret)
        .json(&FederatedEventBatch {
            events: vec![envelope1],
        })
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let second = Event::MemberJoined {
        occurred_at: chrono::Utc::now(),
        workspace_id: ws.id,
        member,
    };
    let (mut envelope2, _) = member_joined_envelope(peer_id, 2, &second, Some(&link1));
    envelope2.event.prev_hash = maidan_types::genesis_hash();
    let ingest = h
        .client
        .post(format!("{}/a2a/v1/events", h.base()))
        .bearer_auth(&peer_secret)
        .json(&FederatedEventBatch {
            events: vec![envelope2],
        })
        .send()
        .await
        .unwrap();
    assert_eq!(ingest.status(), StatusCode::CONFLICT);
    h.shutdown().await;
}

/// Cluster 397.6: a refused event must not wedge the chain.
///
/// The origin chain covers **every** event in the peer's workspace, but
/// federation only accepts the `federatable()` allowlist. The origin link used
/// to be recorded on the ingest path only, so a refused event left the pointer
/// behind it — and every later envelope, whose `prev_hash` chains from the
/// refused one, then failed `PrevHashMismatch` forever. Sequence here is the
/// minimal reproduction: accepted, refused, accepted.
#[tokio::test]
async fn a_refused_event_does_not_wedge_the_origin_chain() {
    let h = spawn().await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "fed-wedge".to_string(),
        })
        .await
        .unwrap();
    let admin = mint_admin_token(h.store.as_ref(), ws.id).await;

    let create = h
        .client
        .post(format!("{}/workspaces/{}/peers", h.base(), ws.id.0))
        .bearer_auth(&admin)
        .json(&json!({ "name": "remote-wedge", "base_url": "https://remote.example" }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = create.json().await.unwrap();
    let peer_id = PeerId(uuid::Uuid::parse_str(body["peer"]["id"].as_str().unwrap()).unwrap());
    let peer_secret = body["secret"].as_str().unwrap().to_string();

    let now = chrono::Utc::now();
    let member = |n: u128| MemberId(uuid::Uuid::from_u128(n));
    // A federatable event and a refused one, from the same producer.
    let federatable = |n: u128| Event::MemberJoined {
        occurred_at: now,
        workspace_id: ws.id,
        member: maidan_types::Member {
            id: member(n),
            workspace_id: ws.id,
            handle: format!("m{n}"),
            display_name: None,
            kind: MemberKind::Agent,
            created_at: now,
            updated_at: now,
            tombstoned_at: None,
        },
    };
    let refused = Event::ArtifactUpserted {
        occurred_at: now,
        artifact: Artifact {
            id: ArtifactId(uuid::Uuid::from_u128(99)),
            sha256: "b".repeat(64),
            size_bytes: 3,
            mime_type: None,
            kind: ArtifactKind::Attachment,
            uploaded_by: None,
            created_at: now,
            tombstoned_at: None,
        },
    };

    // Build a genuine chain: each prev_hash folds its predecessor's link.
    let mut prev = maidan_types::genesis_hash();
    let mut envelopes = Vec::new();
    for (id, (event, kind)) in [
        (federatable(1), EventKind::MemberJoined),
        (refused.clone(), EventKind::ArtifactUpserted),
        (federatable(3), EventKind::MemberJoined),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, e)| (i as i64 + 1, e))
    {
        let payload = serde_json::to_value(&event).unwrap();
        let content_hash = maidan_types::content_hash(&payload).unwrap();
        let stored = StoredEvent {
            id,
            lsn: id,
            kind,
            workspace_id: None,
            channel_id: None,
            thread_id: None,
            payload,
            occurred_at: now,
            prev_hash: prev.clone(),
            content_hash: content_hash.clone(),
        };
        prev = maidan_types::chain_hash(&stored.prev_hash, &content_hash, id);
        envelopes.push(FederationEnvelope {
            origin_peer_id: peer_id,
            remote_event_id: id,
            event: stored,
        });
    }

    // Push them one batch at a time, the way the pull worker walks a log.
    let mut outcomes = Vec::new();
    for envelope in envelopes {
        let res = h
            .client
            .post(format!("{}/a2a/v1/events", h.base()))
            .bearer_auth(&peer_secret)
            .json(&FederatedEventBatch {
                events: vec![envelope],
            })
            .send()
            .await
            .unwrap();
        let status = res.status();
        let body: serde_json::Value = res.json().await.unwrap_or(json!({}));
        outcomes.push((status, body));
    }

    assert_eq!(outcomes[0].0, StatusCode::OK);
    assert_eq!(outcomes[0].1["ingested"], 1);

    assert_eq!(
        outcomes[1].0,
        StatusCode::OK,
        "the refused one is not an error"
    );
    assert_eq!(outcomes[1].1["refused"], 1);
    assert_eq!(outcomes[1].1["ingested"], 0, "and it is not ingested");

    // The one that used to 409 forever.
    assert_eq!(
        outcomes[2].0,
        StatusCode::OK,
        "an event after a refusal must still verify: {:?}",
        outcomes[2].1
    );
    assert_eq!(outcomes[2].1["ingested"], 1, "{:?}", outcomes[2].1);

    h.shutdown().await;
}
