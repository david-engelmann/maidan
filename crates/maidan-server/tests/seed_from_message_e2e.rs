//! Cluster 327: seed-from-message. `POST /messages/:id/seed` spawns a titled child
//! thread linked to the source by a `seeded_from` reference edge; `inclusion=quote`
//! also posts a first message quoting the source. Auth ENABLED (the quote's author
//! is the caller — a NOT-NULL FK).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn seed_spawns_a_linked_child_thread() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("root".into()),
        })
        .await
        .unwrap();
    let source = store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: member.id,
            body: "the tangent starts here".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");
    let seed_url = format!("{base}/messages/{}/seed", source.id.0);

    // Pointer seed: a titled child thread + a seeded_from edge, no content copied.
    let ptr = client
        .post(&seed_url)
        .header("Authorization", &bearer)
        .json(&json!({ "title": "re-ask: before the tangent", "inclusion": "pointer" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ptr.status(), StatusCode::CREATED);
    let child: Value = ptr.json().await.unwrap();
    assert_eq!(child["title"], json!("re-ask: before the tangent"));
    let child_id = child["id"].as_str().unwrap().to_string();

    // The lineage edge is queryable via the reverse relation-filtered query.
    let refs: Value = client
        .get(format!(
            "{base}/references?dst_kind=message&dst_id={}&relation=seeded_from",
            source.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = refs.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["src_id"], json!(child_id));
    assert_eq!(arr[0]["relation"], json!("seeded_from"));

    // Quote seed: a second child whose first message quotes the source.
    let q = client
        .post(&seed_url)
        .header("Authorization", &bearer)
        .json(&json!({ "title": "quoted branch", "inclusion": "quote" }))
        .send()
        .await
        .unwrap();
    assert_eq!(q.status(), StatusCode::CREATED);
    let qchild: Value = q.json().await.unwrap();
    let qctx: Value = client
        .get(format!(
            "{base}/threads/{}/context",
            qchild["id"].as_str().unwrap()
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msgs = qctx["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["body"], json!("> the tangent starts here"));

    // N seeds per source: two seeded_from edges now point at the source.
    let refs: Value = client
        .get(format!(
            "{base}/references?dst_kind=message&dst_id={}&relation=seeded_from",
            source.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(refs.as_array().unwrap().len(), 2);

    // The source thread is untouched (still one message).
    let root_ctx: Value = client
        .get(format!("{base}/threads/{}/context", thread.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(root_ctx["messages"].as_array().unwrap().len(), 1);

    // Validation: bad inclusion + empty title -> 400.
    let bad = client
        .post(&seed_url)
        .header("Authorization", &bearer)
        .json(&json!({ "title": "x", "inclusion": "teleport" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let empty = client
        .post(&seed_url)
        .header("Authorization", &bearer)
        .json(&json!({ "title": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
}
