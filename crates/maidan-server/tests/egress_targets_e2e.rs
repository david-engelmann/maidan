//! The egress trust boundary over HTTP (Cluster 378.1). Auth ENABLED — the whole
//! surface is `token:admin`, and the point of an allowlist is who may change it,
//! so a bypass run would prove nothing.
//!
//! Walks the operator's loop (empty ⇒ deliver nowhere → bless → the
//! authorization check now passes → revoke) and the two refusals that matter: a
//! selector that is a name rather than an id, and a workspace-scoped token.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressSurface, EgressTarget, MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace,
    WorkspaceId,
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
async fn an_operator_blesses_and_revokes_an_egress_target() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let op = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let admin = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![capability::TOKEN_ADMIN.into()],
    )
    .await;
    let targets = format!("{base}/workspaces/{}/egress-targets", ws.id);

    // Default empty: configured with nothing, nothing is trusted.
    let resp = client
        .get(&targets)
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.json::<Vec<Value>>().await.unwrap().is_empty());
    assert!(!store
        .is_egress_target_allowed(ws.id, EgressSurface::Github, "acme/widgets")
        .await
        .unwrap());

    // Bless a repository. 201 + the entry.
    let resp = client
        .post(&targets)
        .bearer_auth(&admin)
        .json(&json!({ "surface": "github", "selector": "acme/widgets" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let entry: Value = resp.json().await.unwrap();
    assert_eq!(entry["surface"], "github");
    assert_eq!(entry["selector"], "acme/widgets");
    assert_eq!(entry["workspace_id"], json!(ws.id.0));
    let id = entry["id"].as_str().unwrap().to_string();

    // The blessing authorizes a delivery to any issue in that repository — the
    // grain a Cluster-379 delivery will check.
    let delivery = EgressTarget::Github {
        repo: "acme/widgets".into(),
        issue_number: 42,
    };
    assert!(store
        .is_egress_target_allowed(ws.id, delivery.surface(), &delivery.allowlist_selector())
        .await
        .unwrap());

    // Re-blessing is idempotent: same entry, still one row.
    let resp = client
        .post(&targets)
        .bearer_auth(&admin)
        .json(&json!({ "surface": "github", "selector": "acme/widgets" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert_eq!(resp.json::<Value>().await.unwrap()["id"], json!(id));
    assert_eq!(
        client
            .get(&targets)
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .json::<Vec<Value>>()
            .await
            .unwrap()
            .len(),
        1
    );

    // A name is not an id, so it is not an allowlist key: 400, not a blessing
    // that silently never matches.
    for selector in ["#general", "general"] {
        let resp = client
            .post(&targets)
            .bearer_auth(&admin)
            .json(&json!({ "surface": "slack", "selector": selector }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "slack selector {selector:?} must be refused"
        );
    }
    let resp = client
        .post(&targets)
        .bearer_auth(&admin)
        .json(&json!({ "surface": "github", "selector": "acme/widgets#42" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "an issue number is not part of the authorization grain"
    );

    // An unknown surface fails at the extractor, so the vocabulary cannot drift.
    let resp = client
        .post(&targets)
        .bearer_auth(&admin)
        .json(&json!({ "surface": "discord", "selector": "C0123ABCDEF" }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_client_error(),
        "an unknown surface is rejected, got {}",
        resp.status()
    );

    // Revoke: 204, the authorization goes away, and a second revoke is a 404.
    let one = format!("{targets}/{id}");
    assert_eq!(
        client
            .delete(&one)
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(!store
        .is_egress_target_allowed(ws.id, EgressSurface::Github, "acme/widgets")
        .await
        .unwrap());
    assert_eq!(
        client
            .delete(&one)
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .delete(format!("{targets}/{}", uuid::Uuid::new_v4()))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND,
        "unknown id"
    );

    // The allowlist is policy, not status: a workspace-scoped token cannot read
    // it (which would hand an agent the list of destinations worth aiming at) and
    // cannot change it.
    let member_token = mint(
        store.as_ref(),
        ws.id,
        op.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
        ],
    )
    .await;
    assert_eq!(
        client
            .get(&targets)
            .bearer_auth(&member_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .post(&targets)
            .bearer_auth(&member_token)
            .json(&json!({ "surface": "slack", "selector": "C0123ABCDEF" }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .delete(&one)
            .bearer_auth(&member_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}

/// A `token:admin` token is scoped to its own workspace, so it cannot bless a
/// destination for another tenant — the allowlist is per workspace and so is the
/// authority over it.
#[tokio::test]
async fn a_workspace_admin_cannot_bless_another_workspaces_target() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let mine = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let theirs = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let op = store
        .create_member(NewMember {
            workspace_id: mine.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let admin = mint(
        store.as_ref(),
        mine.id,
        op.id,
        vec![capability::TOKEN_ADMIN.into()],
    )
    .await;

    let resp = client
        .post(format!("{base}/workspaces/{}/egress-targets", theirs.id))
        .bearer_auth(&admin)
        .json(&json!({ "surface": "slack", "selector": "C0123ABCDEF" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(store
        .list_egress_targets(theirs.id)
        .await
        .unwrap()
        .is_empty());
}
