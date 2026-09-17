//! A WASI handler target can be refused for three different reasons, and only
//! two of them are the caller's fault.
//!
//! `resolve_wasi_handler_target` used to return `Result<String, String>`, so a
//! store that could not answer arrived at the caller with the same shape as a
//! typo'd sha — and both call sites mapped the lot to `400` / `InvalidParams`.
//! A database outage during slash-command registration told the operator their
//! request was malformed.

use maidan_auth::{resolve_wasi_handler_target, AuthContext, WasiTargetError};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewMember, NewWorkspace, WorkspaceId};

async fn store_with_workspace() -> (SqliteStore, sqlx::SqlitePool, WorkspaceId, MemberId) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool.clone());

    let ws = store
        .create_workspace(NewWorkspace {
            name: "wasi-targets".to_string(),
        })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "registrar".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    (store, pool, ws.id, member.id)
}

fn auth(workspace_id: WorkspaceId, member_id: MemberId) -> AuthContext {
    AuthContext::from_session(member_id, workspace_id, maidan_auth::capability::all())
}

#[tokio::test]
async fn a_malformed_target_is_the_callers_fault() {
    let (store, _pool, ws, member) = store_with_workspace().await;
    let err = resolve_wasi_handler_target(&store, &auth(ws, member), ws, "not-a-sha")
        .await
        .expect_err("a malformed target must be refused");
    assert!(
        matches!(err, WasiTargetError::Invalid(_)),
        "expected Invalid, got {err:?}"
    );
}

#[tokio::test]
async fn a_sha_this_workspace_does_not_own_is_the_callers_fault() {
    let (store, _pool, ws, member) = store_with_workspace().await;
    let unowned = "a".repeat(64);
    let err = resolve_wasi_handler_target(&store, &auth(ws, member), ws, &unowned)
        .await
        .expect_err("an unowned sha must be refused");
    match err {
        WasiTargetError::Invalid(msg) => assert!(
            msg.contains("not an artifact of this workspace"),
            "the message must not distinguish absent from unowned: {msg}"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

/// The one that was wrong. Closing the pool makes the ref lookup fail the way a
/// real outage does; the answer must be a store error, which each surface
/// already renders as an internal failure, and not a complaint about the input.
#[tokio::test]
async fn a_store_that_cannot_answer_is_not_a_bad_request() {
    let (store, pool, ws, member) = store_with_workspace().await;
    let sha = "b".repeat(64);
    pool.close().await;

    let err = resolve_wasi_handler_target(&store, &auth(ws, member), ws, &sha)
        .await
        .expect_err("a closed pool cannot resolve a target");
    assert!(
        matches!(err, WasiTargetError::Store(_)),
        "a store failure must not be reported as an invalid target, got {err:?}"
    );
}
