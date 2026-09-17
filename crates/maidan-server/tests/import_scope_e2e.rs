//! A signed bundle proves integrity, never authority.
//!
//! `POST /workspaces/import` writes to the workspace id *inside the bundle*,
//! and that id is caller-supplied. With the documented no-pin default
//! (`MAIDAN_EXPORT_VERIFY_KEYS` unset) an attacker signs with their own key and
//! the envelope verifies, so the signature cannot be the access check. These
//! tests pin the two guards `restore` inherits from `erase_workspace`: the
//! workspace scope, and the legal hold.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, ExportSigningKey, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
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

async fn spawn() -> Harness {
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
    state.attach_export_signing(ExportSigningKey::from_seed(SEED));
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

async fn seed(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId, TokenSecret) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-admin"),
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
            author_id: member.id,
            body: format!("{name} data"),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("admin".into()),
            capabilities: vec![capability::TOKEN_ADMIN.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, member.id, secret)
}

async fn export_of(h: &Harness, ws: WorkspaceId, secret: &TokenSecret) -> Value {
    h.client
        .get(format!("{}/workspaces/{}/export", h.base(), ws.0))
        .header("authorization", format!("Bearer {}", secret.as_str()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The headline: workspace A's `token:admin` cannot restore over workspace B.
/// Before 397.1 this erased B and replaced it, because the handler checked the
/// capability but never whose workspace the bundle named.
#[tokio::test]
async fn an_admin_cannot_restore_over_another_workspace() {
    let h = spawn().await;
    let (ws_a, _, tok_a) = seed(h.store.as_ref(), "alpha").await;
    let (ws_b, _, tok_b) = seed(h.store.as_ref(), "bravo").await;

    // A genuine, correctly-signed bundle for B — the signature is not the issue.
    let bundle_b = export_of(&h, ws_b, &tok_b).await;
    assert_eq!(bundle_b["$type"], "maidan.workspace.export/1");

    let res = h
        .client
        .post(format!(
            "{}/workspaces/import?mode=restore&force=true",
            h.base()
        ))
        .header("authorization", format!("Bearer {}", tok_a.as_str()))
        .json(&bundle_b)
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "A must not restore over B's workspace"
    );

    // B is untouched — the erase never ran.
    let b = h.store.get_workspace(ws_b).await.expect("B still exists");
    assert_eq!(b.name, "bravo");
    // And A is still there too (no collateral).
    assert!(h.store.get_workspace(ws_a).await.is_ok());
    h.shutdown();
}

/// Restoring over your *own* workspace is allowed — the scope check narrows the
/// blast radius without removing the feature.
#[tokio::test]
async fn an_admin_may_restore_over_its_own_workspace() {
    let h = spawn().await;
    let (ws, _, tok) = seed(h.store.as_ref(), "alpha").await;
    let bundle = export_of(&h, ws, &tok).await;

    let res = h
        .client
        .post(format!(
            "{}/workspaces/import?mode=restore&force=true",
            h.base()
        ))
        .header("authorization", format!("Bearer {}", tok.as_str()))
        .json(&bundle)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "self-restore stays supported");
    h.shutdown();
}

/// A force-restore is an erase, so a legal hold refuses it — the same guard
/// `DELETE /workspaces/:id` has. Without this, import was the way around a hold.
#[tokio::test]
async fn a_legal_hold_refuses_a_force_restore() {
    let h = spawn().await;
    let (ws, member, tok) = seed(h.store.as_ref(), "alpha").await;
    let bundle = export_of(&h, ws, &tok).await;
    h.store
        .place_legal_hold(ws, "litigation", Some(member))
        .await
        .unwrap();

    let res = h
        .client
        .post(format!(
            "{}/workspaces/import?mode=restore&force=true",
            h.base()
        ))
        .header("authorization", format!("Bearer {}", tok.as_str()))
        .json(&bundle)
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::CONFLICT,
        "a hold blocks the erase"
    );
    assert!(h.store.get_workspace(ws).await.is_ok());
    h.shutdown();
}
