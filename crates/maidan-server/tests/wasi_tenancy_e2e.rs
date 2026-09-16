//! Clusters 399.2 + 399.4: a WASI handler runs only over a module its own
//! workspace owns — refused at registration, and still refused at dispatch.
//!
//! Auth is **enabled** here on purpose. Under `AUTH_DISABLED` the upload path
//! records no Cluster-204 access links at all, so ownership is not a thing that
//! exists to test — the sibling `wasi_slash_e2e` covers the run path in that
//! mode, and this covers the boundary in the mode that has one.
//!
//! Without the check, a registration could name any sha on the instance and turn
//! a slash command into a cross-tenant artifact reader — it executes the bytes,
//! so it would also be a cross-tenant *code* reader.
//!
//! **Two gates, two questions.** Registration asks "can this ever run?" and
//! refuses a configuration that could never be honoured. Dispatch asks "may this
//! run *now*?" and has to keep asking, because a workspace can lose an artifact
//! after the registration was accepted. Neither subsumes the other, and this
//! file asserts both — the second by taking the access link away from a
//! registration that was legitimately accepted.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime, SlashRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

fn echo_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
             (import "wasi_snapshot_preview1" "fd_write"
               (func $fd_write (param i32 i32 i32 i32) (result i32)))
             (memory (export "memory") 1)
             (data (i32.const 8) "ran")
             (func (export "_start")
               (i32.store (i32.const 0) (i32.const 8))
               (i32.store (i32.const 4) (i32.const 3))
               (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 40)))))"#,
    )
    .expect("valid wat")
}

struct Tenant {
    ws: WorkspaceId,
    thread: maidan_types::ThreadId,
    author: MemberId,
    token: String,
}

#[tokio::test]
async fn a_handler_cannot_run_another_tenants_module() {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED — the whole point
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.slash = SlashRuntime::new(Some(Arc::new([9u8; 32])));
    std::mem::forget(dir);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let mut tenants = Vec::new();
    for name in ["owner", "stranger"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let author = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: format!("{name}-m"),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "c".into(),
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
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id: author.id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![
                    capability::WORKSPACE_READ.into(),
                    capability::WORKSPACE_WRITE.into(),
                    capability::MESSAGE_POST.into(),
                    capability::ARTIFACT_UPLOAD.into(),
                ],
                expires_at: None,
            })
            .await
            .unwrap();
        tenants.push(Tenant {
            ws: ws.id,
            thread: thread.id,
            author: author.id,
            token: secret.as_str().to_string(),
        });
    }
    let (owner, stranger) = (&tenants[0], &tenants[1]);

    // The owner uploads the module — this is what records the Cluster-204 ref.
    let upload: Value = client
        .post(format!("{base}/artifacts?kind=attachment"))
        .bearer_auth(&owner.token)
        .body(echo_module())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sha = upload["sha256"].as_str().expect("sha256").to_string();

    let register = |t: &Tenant, name: &'static str| {
        let (base, token, ws, sha) = (base.clone(), t.token.clone(), t.ws, sha.clone());
        let client = client.clone();
        async move {
            client
                .post(format!("{base}/workspaces/{}/slash-commands", ws.0))
                .bearer_auth(token)
                .json(&json!({ "name": name, "handler_kind": "wasi", "handler_target": sha }))
                .send()
                .await
                .unwrap()
        }
    };
    assert_eq!(
        register(owner, "mine").await.status(),
        reqwest::StatusCode::CREATED,
        "the workspace that owns the bytes may register them"
    );

    // Gate one: the stranger cannot even register a handler over the owner's
    // sha. Refused here rather than accepted-and-permanently-broken.
    let refused = register(stranger, "theirs").await;
    assert_eq!(
        refused.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "registering another tenant's sha must be refused at registration"
    );
    let why = refused.text().await.unwrap_or_default();
    assert!(
        why.contains("not an artifact of this workspace"),
        "the refusal should say what is wrong, got: {why}"
    );
    assert!(
        !why.contains(&sha),
        "the refusal must not echo the sha back as confirmation it exists: {why}"
    );

    let invoke = |t: &Tenant, body: &'static str| {
        let (base, token, thread, author) = (base.clone(), t.token.clone(), t.thread, t.author);
        let client = client.clone();
        async move {
            client
                .post(format!("{base}/threads/{}/messages", thread.0))
                .bearer_auth(token)
                .json(&json!({ "author_id": author.0, "body": body }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    // The owner's handler runs.
    let mine = invoke(owner, "/mine").await;
    let mine_resp = &mine["metadata"]["slash_response"];
    assert_eq!(mine_resp["ok"], true, "owner should run: {mine_resp:?}");
    assert_eq!(mine_resp["response"]["content"][0]["text"], "ran");

    // The stranger's command was never created, so `/theirs` is not a command at
    // all — it posts as ordinary text with no slash metadata, which is what an
    // unregistered name has always done.
    let theirs = invoke(stranger, "/theirs").await;
    assert!(
        theirs["metadata"]["slash_response"].is_null(),
        "the refused registration must not have been persisted: {theirs:?}"
    );

    // Gate two: a registration that was legitimately accepted still has to pass
    // at dispatch, because ownership can go away afterwards. Drop the
    // Cluster-204 access link out from under the owner's own command.
    sqlx::query("DELETE FROM maidan_artifact_refs WHERE workspace_id = ? AND sha256 = ?")
        .bind(owner.ws.0)
        .bind(&sha)
        .execute(&pool)
        .await
        .unwrap();

    let orphaned = invoke(owner, "/mine").await;
    let orphaned_resp = &orphaned["metadata"]["slash_response"];
    assert_eq!(
        orphaned_resp["ok"], false,
        "losing the artifact must stop the handler: {orphaned_resp:?}"
    );
    assert_eq!(orphaned_resp["error_kind"], "invalid_module");
    assert_eq!(
        orphaned_resp["stdout"], "",
        "the guest must not have run at all"
    );
}
