//! Operator lifecycle for cross-organization share tickets over REST. Auth is
//! enabled: only `token:admin` may issue/list/revoke, secrets appear once, and
//! neither the persisted hash nor plaintext secret reaches lists or audit.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use chrono::{Duration as ChronoDuration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ArtifactKind, MemberId, MemberKind, NewApiToken, NewArtifact, NewChannel, NewMember,
    NewWorkspace, WorkspaceId,
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
    secret.as_str().to_owned()
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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn share_ticket_operator_lifecycle_is_scoped_and_secret_safe() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "owner".into(),
        })
        .await
        .unwrap();
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "incident".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let sha = "a".repeat(64);
    store
        .upsert_artifact(NewArtifact {
            sha256: sha.clone(),
            size_bytes: 7,
            mime_type: Some("text/plain".into()),
            kind: ArtifactKind::Attachment,
            uploaded_by: Some(owner.id),
        })
        .await
        .unwrap();
    store.record_artifact_ref(ws.id, &sha).await.unwrap();

    let admin = mint(
        store.as_ref(),
        ws.id,
        owner.id,
        vec![capability::TOKEN_ADMIN.into()],
    )
    .await;
    let auth =
        |req: reqwest::RequestBuilder| req.header("authorization", format!("Bearer {admin}"));
    let create = auth(
        client
            .post(format!("{base}/workspaces/{}/share-tickets", ws.id.0))
            .json(&json!({
                "channel_id": channel.id.0,
                "owner_id": owner.id.0,
                "expires_at": Utc::now() + ChronoDuration::hours(24),
                "artifact_shas": [sha],
            })),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let created: Value = create.json().await.unwrap();
    let ticket_id = created["ticket"]["id"].as_str().unwrap();
    let secret = created["secret"].as_str().unwrap();
    assert!(secret.starts_with("maid_share_"));
    assert_eq!(created["artifact_shas"], json!([sha]));
    assert!(created["ticket"].get("token_hash").is_none());

    let list = auth(client.get(format!("{base}/workspaces/{}/share-tickets", ws.id.0)))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let listed: Value = list.json().await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let serialized_list = listed.to_string();
    assert!(!serialized_list.contains(secret));
    assert!(!serialized_list.contains("token_hash"));

    let audit = store.list_audit_for_workspace(ws.id, 20).await.unwrap();
    let serialized_audit = serde_json::to_string(&audit).unwrap();
    assert!(serialized_audit.contains("share_ticket.create"));
    assert!(!serialized_audit.contains(secret));
    assert!(!serialized_audit.contains(&hash_secret(secret)));

    let plain = mint(
        store.as_ref(),
        ws.id,
        owner.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;
    let denied = client
        .get(format!("{base}/workspaces/{}/share-tickets", ws.id.0))
        .header("authorization", format!("Bearer {plain}"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let cross_workspace =
        auth(client.get(format!("{base}/workspaces/{}/share-tickets", other_ws.id.0)))
            .send()
            .await
            .unwrap();
    assert_eq!(cross_workspace.status(), StatusCode::FORBIDDEN);

    let revoke = auth(client.delete(format!(
        "{base}/workspaces/{}/share-tickets/{ticket_id}",
        ws.id.0
    )))
    .send()
    .await
    .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
    let revoked = store
        .get_share_ticket(maidan_types::ShareTicketId(ticket_id.parse().unwrap()))
        .await
        .unwrap();
    assert!(revoked.revoked_at.is_some());
}
