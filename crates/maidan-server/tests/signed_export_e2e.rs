//! Cluster 391: signed workspace export a blank instance can verify
//! without the origin host. Tokens die on export.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, ExportSigningKey, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const SEED: [u8; 32] = [0x11; 32];

struct Harness {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn(sign: bool, verify_keys: Vec<[u8; 32]>) -> Harness {
    let pool = SqlitePoolOptions::new()
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
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    if sign {
        state.attach_export_signing(ExportSigningKey::from_seed(SEED));
    }
    if !verify_keys.is_empty() {
        state.attach_export_verify_keys(verify_keys);
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        client: reqwest::Client::new(),
        store,
        server,
    }
}

async fn mint_admin(
    store: &dyn Store,
    ws: WorkspaceId,
    member: maidan_types::MemberId,
) -> TokenSecret {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("admin".into()),
            capabilities: vec![capability::TOKEN_ADMIN.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret
}

async fn seed_room(store: &dyn Store) -> (WorkspaceId, maidan_types::MemberId, TokenSecret) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "origin".into(),
        })
        .await
        .unwrap();
    let alice = store
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
            title: Some("t".into()),
        })
        .await
        .unwrap();
    store
        .post_message(NewMessage {
            thread_id: thread.id,
            author_id: alice.id,
            body: "hello signed".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let secret = mint_admin(store, ws.id, alice.id).await;
    (ws.id, alice.id, secret)
}

#[tokio::test]
async fn blank_instance_verifies_and_imports_without_origin() {
    let origin = spawn(true, Vec::new()).await;
    let (ws, member, secret) = seed_room(origin.store.as_ref()).await;
    let auth = format!("Bearer {}", secret.as_str());

    let envelope: Value = origin
        .client
        .get(format!("{}/workspaces/{}/export", origin.base(), ws.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(envelope["$type"], "maidan.workspace.export/1");
    assert_eq!(envelope["token_policy"], "tokens_die_on_export");

    // Destination: blank store, no signing key, no origin callback.
    let dest = spawn(false, Vec::new()).await;
    let dest_ws = dest
        .store
        .create_workspace(NewWorkspace {
            name: "dest-admin".into(),
        })
        .await
        .unwrap();
    let dest_admin = dest
        .store
        .create_member(NewMember {
            workspace_id: dest_ws.id,
            handle: "ops".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let dest_secret = mint_admin(dest.store.as_ref(), dest_ws.id, dest_admin.id).await;
    let dest_auth = format!("Bearer {}", dest_secret.as_str());

    let verified = dest
        .client
        .post(format!("{}/workspaces/export/verify", dest.base()))
        .header("authorization", &dest_auth)
        .json(&envelope)
        .send()
        .await
        .unwrap();
    assert_eq!(verified.status(), StatusCode::OK);
    let verify_body: Value = verified.json().await.unwrap();
    assert_eq!(verify_body["ok"], true);
    assert_eq!(verify_body["token_policy"], "tokens_die_on_export");
    assert_eq!(
        verify_body["workspace_id"].as_str().unwrap(),
        ws.0.to_string()
    );

    let imported = dest
        .client
        .post(format!("{}/workspaces/import?mode=new", dest.base()))
        .header("authorization", &dest_auth)
        .json(&envelope)
        .send()
        .await
        .unwrap();
    assert_eq!(imported.status(), StatusCode::OK);
    let result: Value = imported.json().await.unwrap();
    let new_ws = WorkspaceId(result["workspace_id"].as_str().unwrap().parse().unwrap());
    let dest_members = dest.store.list_members(new_ws).await.unwrap();
    assert_eq!(dest_members.len(), 1);
    assert_eq!(dest_members[0].handle, "alice");
    let dest_tokens = dest
        .store
        .list_api_tokens_for_member(new_ws, dest_members[0].id)
        .await
        .unwrap();
    assert!(
        dest_tokens.is_empty(),
        "tokens die on export — imported member has no tokens"
    );

    // Origin bearer is unknown on the blank instance.
    let rejected = dest
        .client
        .get(format!("{}/workspaces/{}/export", dest.base(), new_ws.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

    let origin_tokens = origin
        .store
        .list_api_tokens_for_member(ws, member)
        .await
        .unwrap();
    assert_eq!(origin_tokens.len(), 1);

    origin.shutdown();
    dest.shutdown();
}

#[tokio::test]
async fn bit_flip_and_bad_signature_fail_closed() {
    let origin = spawn(true, Vec::new()).await;
    let (ws, _, secret) = seed_room(origin.store.as_ref()).await;
    let auth = format!("Bearer {}", secret.as_str());
    let mut envelope: Value = origin
        .client
        .get(format!("{}/workspaces/{}/export", origin.base(), ws.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    envelope["payload"]["messages"][0]["body"] = json!("tampered");
    let flipped = origin
        .client
        .post(format!("{}/workspaces/export/verify", origin.base()))
        .header("authorization", &auth)
        .json(&envelope)
        .send()
        .await
        .unwrap();
    assert_eq!(flipped.status(), StatusCode::BAD_REQUEST);

    let mut signed: Value = origin
        .client
        .get(format!("{}/workspaces/{}/export", origin.base(), ws.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut sig = signed["signature"].as_str().unwrap().to_string();
    let flipped_nibble = if sig.as_bytes()[0] == b'0' { '1' } else { '0' };
    sig.replace_range(0..1, &flipped_nibble.to_string());
    signed["signature"] = json!(sig);
    let bad_sig = origin
        .client
        .post(format!("{}/workspaces/export/verify", origin.base()))
        .header("authorization", &auth)
        .json(&signed)
        .send()
        .await
        .unwrap();
    assert_eq!(bad_sig.status(), StatusCode::BAD_REQUEST);

    origin.shutdown();
}

#[tokio::test]
async fn verify_key_pin_rejects_a_stranger_key() {
    let origin = spawn(true, Vec::new()).await;
    let (ws, _, secret) = seed_room(origin.store.as_ref()).await;
    let auth = format!("Bearer {}", secret.as_str());
    let envelope: Value = origin
        .client
        .get(format!("{}/workspaces/{}/export", origin.base(), ws.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let other = ExportSigningKey::from_seed([0x22; 32]);
    let dest = spawn(false, vec![other.public_key_bytes()]).await;
    let dest_ws = dest
        .store
        .create_workspace(NewWorkspace {
            name: "dest".into(),
        })
        .await
        .unwrap();
    let dest_admin = dest
        .store
        .create_member(NewMember {
            workspace_id: dest_ws.id,
            handle: "ops".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let dest_secret = mint_admin(dest.store.as_ref(), dest_ws.id, dest_admin.id).await;
    let dest_auth = format!("Bearer {}", dest_secret.as_str());
    let denied = dest
        .client
        .post(format!("{}/workspaces/export/verify", dest.base()))
        .header("authorization", &dest_auth)
        .json(&envelope)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);

    origin.shutdown();
    dest.shutdown();
}

#[tokio::test]
async fn export_without_signing_key_refuses() {
    let h = spawn(false, Vec::new()).await;
    let (ws, _, secret) = seed_room(h.store.as_ref()).await;
    let resp = h
        .client
        .get(format!("{}/workspaces/{}/export", h.base(), ws.0))
        .header("authorization", format!("Bearer {}", secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    h.shutdown();
}

#[tokio::test]
async fn operator_public_key_matches_the_signer() {
    let h = spawn(true, Vec::new()).await;
    let (ws, _, secret) = seed_room(h.store.as_ref()).await;
    let auth = format!("Bearer {}", secret.as_str());
    let pk: Value = h
        .client
        .get(format!("{}/operator/export-public-key", h.base()))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let envelope: Value = h
        .client
        .get(format!("{}/workspaces/{}/export", h.base(), ws.0))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pk["alg"], "ed25519");
    assert_eq!(pk["token_policy"], "tokens_die_on_export");
    assert_eq!(pk["public_key"], envelope["public_key"]);
    h.shutdown();
}
