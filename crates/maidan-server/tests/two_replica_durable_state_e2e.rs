//! Cross-replica durable ephemeral state (Cluster 104.0.4).
//!
//! Two HTTP servers sharing one Postgres database stand in for two server
//! replicas behind a load balancer. Because OAuth authorization codes and
//! reindex job status now live in the store (Clusters 104.0.1–.3) rather than
//! per-replica memory, an authorization code minted on replica A can be
//! exchanged on replica B, and a reindex job started on replica A is visible
//! from replica B — neither of which the old in-memory maps could do.

use std::net::SocketAddr;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;

use maidan_auth::{capability, hash_secret};
use maidan_search::{PostgresSearch, Search};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace, WorkspaceId,
};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Build a replica server over `store`/`search` and return its bound address.
fn spawn_replica(store: Arc<dyn Store>, search: Arc<dyn Search>) -> SocketAddr {
    let dir = std::env::temp_dir().join(format!("maidan-104-{}", uuid::Uuid::new_v4()));
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store,
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
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

async fn seed_admin_token(store: &dyn Store) -> (WorkspaceId, String) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "xrep-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = maidan_auth::TokenSecret::generate();
    store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::TOKEN_ADMIN.into(),
                capability::WORKSPACE_WRITE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, secret.as_str().to_string())
}

#[tokio::test]
async fn oauth_code_and_reindex_job_cross_replicas() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping two-replica durable-state e2e: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();

    // One store + search, shared by both replicas (one database, two servers).
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn Search> = Arc::new(PostgresSearch::new(pool.clone()));
    maidan_server::metrics::init();
    let replica_a = spawn_replica(store.clone(), search.clone());
    let replica_b = spawn_replica(store.clone(), search.clone());

    let (wid, admin_secret) = seed_admin_token(store.as_ref()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let auth = format!("Bearer {admin_secret}");
    let wid = wid.0.to_string();

    // --- OAuth: mint on replica A, exchange on replica B ---
    let app_resp = client
        .post(format!("http://{replica_a}/workspaces/{wid}/apps"))
        .header("Authorization", &auth)
        .json(&json!({"slug": "xrep-bot", "name": "Cross-Replica Bot"}))
        .send()
        .await
        .unwrap();
    assert_eq!(app_resp.status(), 201);
    let app_id = app_resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let authz = client
        .post(format!(
            "http://{replica_a}/workspaces/{wid}/apps/{app_id}/oauth/authorize"
        ))
        .header("Authorization", &auth)
        .json(&json!({"redirect_uri": "https://app.example/cb", "state": "s"}))
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status(), 201);
    let code = authz.json::<serde_json::Value>().await.unwrap()["authorization_code"]
        .as_str()
        .unwrap()
        .to_string();

    // Exchange on the OTHER replica — only possible because the code is in the store.
    let exch = client
        .post(format!("http://{replica_b}/oauth/app/token"))
        .json(&json!({"code": code, "redirect_uri": "https://app.example/cb"}))
        .send()
        .await
        .unwrap();
    assert_eq!(exch.status(), 201, "code minted on A must exchange on B");
    let app_secret = exch.json::<serde_json::Value>().await.unwrap()["secret"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(maidan_auth::resolve_bearer(store.as_ref(), &app_secret)
        .await
        .is_ok());

    // --- Reindex: start on replica A, observe status on replica B ---
    // A live message gives the workspace-scoped reindex something to process.
    let bot = store
        .create_member(NewMember {
            workspace_id: WorkspaceId(uuid::Uuid::parse_str(&wid).unwrap()),
            handle: "indexed-bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: WorkspaceId(uuid::Uuid::parse_str(&wid).unwrap()),
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let th = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: th.id,
            author_id: bot.id,
            body: "cross replica reindex".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();

    let started = client
        .post(format!("http://{replica_a}/operator/reindex-embeddings"))
        .header("Authorization", &auth)
        .json(&json!({"workspace_id": wid}))
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), 202);
    let job_id = started.json::<serde_json::Value>().await.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Poll the OTHER replica until the job reaches a terminal state.
    let mut completed = false;
    for _ in 0..100 {
        let job: serde_json::Value = client
            .get(format!(
                "http://{replica_b}/operator/reindex-embeddings/{job_id}"
            ))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if job["status"] == "completed" {
            assert_eq!(job["processed"], 1);
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        completed,
        "job started on A must complete-and-be-visible on B"
    );
}
