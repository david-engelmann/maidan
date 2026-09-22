//! A share ticket can read exactly one channel and its explicit artifact
//! allowlist through the dedicated consumer router—and nothing else.

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
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
    WorkspaceId,
};
use reqwest::{header, StatusCode};
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    (addr, client, store)
}

async fn upload(client: &reqwest::Client, base: &str, bearer: &str, body: &'static [u8]) -> String {
    let response = client
        .post(format!(
            "{base}/artifacts?kind=attachment&mime_type=text/plain"
        ))
        .header("authorization", format!("Bearer {bearer}"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response.json::<Value>().await.unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn consumer_surface_is_paginated_read_only_and_fail_closed() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "owner".into(),
        })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let incident = store
        .create_channel(NewChannel {
            workspace_id: workspace.id,
            name: "incident".into(),
            topic: Some("shared room".into()),
            private: true,
        })
        .await
        .unwrap();
    let unrelated = store
        .create_channel(NewChannel {
            workspace_id: workspace.id,
            name: "unrelated".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let first_thread = store
        .create_thread(NewThread {
            channel_id: incident.id,
            parent_thread_id: None,
            title: Some("first".into()),
        })
        .await
        .unwrap();
    let second_thread = store
        .create_thread(NewThread {
            channel_id: incident.id,
            parent_thread_id: None,
            title: Some("second".into()),
        })
        .await
        .unwrap();
    let unrelated_thread = store
        .create_thread(NewThread {
            channel_id: unrelated.id,
            parent_thread_id: None,
            title: Some("secret".into()),
        })
        .await
        .unwrap();
    for body in ["first message", "second message"] {
        store
            .post_message(NewMessage {
                thread_id: first_thread.id,
                author_id: owner.id,
                body: body.into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
    }
    store
        .post_message(NewMessage {
            thread_id: unrelated_thread.id,
            author_id: owner.id,
            body: "must not leak".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();

    let admin = mint(
        store.as_ref(),
        workspace.id,
        owner.id,
        vec![
            capability::TOKEN_ADMIN.into(),
            capability::ARTIFACT_UPLOAD.into(),
            capability::WORKSPACE_READ.into(),
        ],
    )
    .await;
    let allowed_sha = upload(&client, &base, &admin, b"allowed bytes").await;
    let denied_sha = upload(&client, &base, &admin, b"same workspace, not selected").await;

    let create = client
        .post(format!(
            "{base}/workspaces/{}/share-tickets",
            workspace.id.0
        ))
        .header("authorization", format!("Bearer {admin}"))
        .json(&json!({
            "channel_id": incident.id.0,
            "owner_id": owner.id.0,
            "expires_at": Utc::now() + ChronoDuration::hours(1),
            "artifact_shas": [allowed_sha],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let created: Value = create.json().await.unwrap();
    let ticket_id = created["ticket"]["id"].as_str().unwrap();
    let share_secret = created["secret"].as_str().unwrap().to_owned();
    let share_auth = format!("ShareTicket {share_secret}");

    let manifest_response = client
        .get(format!("{base}/share/manifest"))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(manifest_response.status(), StatusCode::OK);
    assert_eq!(
        manifest_response.headers()[header::CACHE_CONTROL],
        "no-store"
    );
    assert_eq!(
        manifest_response.headers()[header::REFERRER_POLICY],
        "no-referrer"
    );
    let manifest: Value = manifest_response.json().await.unwrap();
    assert_eq!(manifest["channel"]["id"], incident.id.0.to_string());
    assert_eq!(manifest["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["artifacts"][0]["sha256"], allowed_sha);
    assert!(!manifest.to_string().contains(&denied_sha));

    let first_page: Value = client
        .get(format!("{base}/share/threads?limit=1"))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first_page["items"].as_array().unwrap().len(), 1);
    let thread_cursor = first_page["next_cursor"].as_str().unwrap();
    let second_page: Value = client
        .get(format!(
            "{base}/share/threads?limit=1&cursor={thread_cursor}"
        ))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second_page["items"].as_array().unwrap().len(), 1);
    let shared_thread_ids = [
        first_page["items"][0]["id"].as_str().unwrap(),
        second_page["items"][0]["id"].as_str().unwrap(),
    ];
    let first_thread_id = first_thread.id.0.to_string();
    let second_thread_id = second_thread.id.0.to_string();
    assert!(shared_thread_ids.contains(&first_thread_id.as_str()));
    assert!(shared_thread_ids.contains(&second_thread_id.as_str()));
    assert!(!first_page.to_string().contains("claim_lease_id"));

    let messages: Value = client
        .get(format!(
            "{base}/share/threads/{}/messages?limit=1",
            first_thread.id.0
        ))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(messages["items"].as_array().unwrap().len(), 1);
    assert!(messages["next_cursor"].is_string());
    let cross_channel = client
        .get(format!(
            "{base}/share/threads/{}/messages",
            unrelated_thread.id.0
        ))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(cross_channel.status(), StatusCode::NOT_FOUND);

    let allowed = client
        .get(format!("{base}/share/artifacts/{allowed_sha}"))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(allowed.bytes().await.unwrap().as_ref(), b"allowed bytes");
    let denied = client
        .get(format!("{base}/share/artifacts/{denied_sha}"))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);

    // Credential types cannot cross: a share secret is not an API bearer and
    // an API token is not a share ticket.
    let ordinary = client
        .get(format!("{base}/workspaces/{}", workspace.id.0))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(ordinary.status(), StatusCode::UNAUTHORIZED);
    let bearer_share = client
        .get(format!("{base}/share/manifest"))
        .header("authorization", format!("Bearer {share_secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(bearer_share.status(), StatusCode::UNAUTHORIZED);
    let api_as_share = client
        .get(format!("{base}/share/manifest"))
        .header("authorization", format!("ShareTicket {admin}"))
        .send()
        .await
        .unwrap();
    assert_eq!(api_as_share.status(), StatusCode::UNAUTHORIZED);

    let invalid = client
        .get(format!("{base}/share/manifest"))
        .header(
            "authorization",
            format!("ShareTicket maid_share_{}", "f".repeat(64)),
        )
        .send()
        .await
        .unwrap();
    let invalid_status = invalid.status();
    let invalid_body = invalid.text().await.unwrap();
    let revoke = client
        .delete(format!(
            "{base}/workspaces/{}/share-tickets/{ticket_id}",
            workspace.id.0
        ))
        .header("authorization", format!("Bearer {admin}"))
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
    let revoked = client
        .get(format!("{base}/share/manifest"))
        .header("authorization", &share_auth)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), invalid_status);
    assert_eq!(revoked.text().await.unwrap(), invalid_body);

    let expiring = client
        .post(format!(
            "{base}/workspaces/{}/share-tickets",
            workspace.id.0
        ))
        .header("authorization", format!("Bearer {admin}"))
        .json(&json!({
            "channel_id": incident.id.0,
            "owner_id": owner.id.0,
            "expires_at": Utc::now() + ChronoDuration::seconds(1),
            "artifact_shas": [],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(expiring.status(), StatusCode::CREATED);
    let expiring_secret = expiring.json::<Value>().await.unwrap()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    let expired = client
        .get(format!("{base}/share/manifest"))
        .header("authorization", format!("ShareTicket {expiring_secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(expired.status(), invalid_status);
    assert_eq!(expired.text().await.unwrap(), invalid_body);
}
