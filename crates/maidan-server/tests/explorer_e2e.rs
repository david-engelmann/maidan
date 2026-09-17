//! Tombstone explorer, message backlinks, EventKind census REST. Auth ENABLED
//! with a minted `workspace:read` bearer.

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
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewPin, NewReaction,
    NewReference, NewThread, NewVote, NewWorkspace, RefSide, RelationKind, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::Value;
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
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

fn new_msg(thread_id: maidan_types::ThreadId, author: MemberId, body: &str) -> NewMessage {
    NewMessage {
        thread_id,
        author_id: author,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    }
}

#[tokio::test]
async fn explorer_rest_lists_tombstones_backlinks_and_census() {
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

    let (ws, _) = store
        .create_workspace_with_event(NewWorkspace { name: "ex".into() })
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
    let (public, _) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "pub".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let (private, _) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "priv".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let (pub_th, _) = store
        .create_thread_with_event(NewThread {
            channel_id: public.id,
            parent_thread_id: None,
            title: Some("pub".into()),
        })
        .await
        .unwrap();
    let (priv_th, _) = store
        .create_thread_with_event(NewThread {
            channel_id: private.id,
            parent_thread_id: None,
            title: Some("secret".into()),
        })
        .await
        .unwrap();

    let (keep, _) = store
        .post_message_with_event(new_msg(pub_th.id, member.id, "keep"), None)
        .await
        .unwrap();
    let (dead, _) = store
        .post_message_with_event(new_msg(pub_th.id, member.id, "drop"), None)
        .await
        .unwrap();
    store
        .tombstone_message_with_event(dead.id, None)
        .await
        .unwrap();
    let (hidden, _) = store
        .post_message_with_event(new_msg(priv_th.id, member.id, "secret"), None)
        .await
        .unwrap();
    store
        .tombstone_message_with_event(hidden.id, None)
        .await
        .unwrap();

    let (src, _) = store
        .post_message_with_event(new_msg(pub_th.id, member.id, "src"), None)
        .await
        .unwrap();
    store
        .add_reference_with_event(NewReference {
            src_kind: RefSide::Message,
            src_id: src.id.0,
            dst_kind: RefSide::Message,
            dst_id: keep.id.0,
            relation: RelationKind::Supports,
        })
        .await
        .unwrap();
    store
        .pin_message_with_event(NewPin {
            thread_id: pub_th.id,
            message_id: keep.id,
            member_id: member.id,
        })
        .await
        .unwrap();
    store
        .add_reaction_with_event(NewReaction {
            message_id: keep.id,
            member_id: member.id,
            emoji: "👍".into(),
        })
        .await
        .unwrap();
    store
        .cast_vote_with_event(NewVote {
            message_id: keep.id,
            member_id: member.id,
            kind: "up".into(),
            confidence: None,
        })
        .await
        .unwrap();

    let tok = mint(store.as_ref(), ws.id, member.id).await;
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
    let bearer = format!("Bearer {tok}");

    let unauth = client
        .get(format!("{base}/workspaces/{}/tombstones", ws.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    let tombs: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/tombstones", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<String> = tombs
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&dead.id.0.to_string()));
    assert!(
        !ids.contains(&hidden.id.0.to_string()),
        "private-channel tombstone must not leak"
    );
    assert!(tombs.iter().all(|t| t["retained"] == true));

    store.purge_message(dead.id).await.unwrap();
    let after: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/tombstones", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(after.iter().all(|t| t["id"] != dead.id.0.to_string()));
    let purged: Vec<Value> = client
        .get(format!(
            "{base}/workspaces/{}/tombstones?include_purged=true",
            ws.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let purged_ids: Vec<String> = purged
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(purged_ids.contains(&dead.id.0.to_string()));
    let dead_row = purged
        .iter()
        .find(|t| t["id"] == dead.id.0.to_string())
        .unwrap();
    assert_eq!(dead_row["retained"], false);

    let back: Value = client
        .get(format!("{base}/messages/{}/backlinks", keep.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(back["message_id"], keep.id.0.to_string());
    assert_eq!(back["references"].as_array().unwrap().len(), 1);
    assert_eq!(back["references"][0]["src_id"], src.id.0.to_string());
    assert_eq!(back["pins"].as_array().unwrap().len(), 1);
    assert_eq!(back["reactions"].as_array().unwrap().len(), 1);
    assert_eq!(back["votes"].as_array().unwrap().len(), 1);

    // Incoming ref points at the (now purged) dead message — 404 after purge.
    let gone = client
        .get(format!("{base}/messages/{}/backlinks", dead.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    let census: Value = client
        .get(format!("{base}/workspaces/{}/kind-census", ws.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(census["workspace_id"], ws.id.0.to_string());
    assert!(census["total"].as_i64().unwrap() > 0);
    let kinds: Vec<&str> = census["counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"message_posted"));
    assert!(kinds.contains(&"workspace_created"));
}
