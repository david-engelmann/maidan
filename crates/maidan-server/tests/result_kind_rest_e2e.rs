//! Cluster 381.2: `GET /workspaces/:id/results?result_kind=` — exact-match
//! facet on the namespaced string. Auth ENABLED with a minted bearer.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const REVIEW: &str = "pi.review.result/1";
const PLAN: &str = "pi.plan.result/1";

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn list_workspace_results_filters_by_namespaced_kind() {
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
        .create_workspace(NewWorkspace { name: "rk".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "me".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let public = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "pub".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let private = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "priv".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();

    let review = store
        .create_thread(NewThread {
            channel_id: public.id,
            parent_thread_id: None,
            title: Some("review".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_result(
            review.id,
            member.id,
            &json!({ "result_kind": REVIEW, "status": "reviewed" }),
        )
        .await
        .unwrap();
    let plan = store
        .create_thread(NewThread {
            channel_id: public.id,
            parent_thread_id: None,
            title: Some("plan".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_result(
            plan.id,
            member.id,
            &json!({ "result_kind": PLAN, "status": "reviewed" }),
        )
        .await
        .unwrap();
    let hidden = store
        .create_thread(NewThread {
            channel_id: private.id,
            parent_thread_id: None,
            title: Some("secret review".into()),
        })
        .await
        .unwrap();
    store
        .set_thread_result(
            hidden.id,
            member.id,
            &json!({ "result_kind": REVIEW, "status": "reviewed" }),
        )
        .await
        .unwrap();

    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let state = AppState::new(
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
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let bearer = format!("Bearer {tok}");

    let unauth = client
        .get(format!("{base}/workspaces/{}/results", ws.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    let all: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/results", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<String> = all
        .iter()
        .map(|r| r["thread_id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&review.id.0.to_string()));
    assert!(ids.contains(&plan.id.0.to_string()));
    assert!(
        !ids.contains(&hidden.id.0.to_string()),
        "a private-channel result the caller cannot access must not leak"
    );
    assert_eq!(all.len(), 2);

    let reviews: Vec<Value> = client
        .get(format!(
            "{base}/workspaces/{}/results?result_kind={REVIEW}",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["thread_id"], json!(review.id.0));
    assert_eq!(reviews[0]["result"]["result_kind"], REVIEW);

    let none: Vec<Value> = client
        .get(format!(
            "{base}/workspaces/{}/results?result_kind=decision",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        none.is_empty(),
        "the old closed-enum word is not a namespaced kind"
    );
}
