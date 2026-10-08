//! A thread's version and its linked artifacts, on both backends. Every write
//! to what a reviewer reads moves the version, whichever path makes it; a write
//! that changes none of it does not. Only an artifact the thread's workspace
//! holds can be linked.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const HELD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ELSEWHERE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn workspace(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId, ThreadId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace")
        .id;
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member")
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel")
        .id;
    let thread = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
        })
        .await
        .expect("thread")
        .id;
    (ws, member, thread)
}

fn erase_audit(erasure: &ArtifactErasure) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(erasure.workspace_id),
        actor_id: None,
        action: "artifact.erase".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(erasure.workspace_id.0),
        metadata: json!({ "last_reference": erasure.last_reference }),
    }
}

async fn run_suite(store: &dyn Store) {
    let (ws, worker, thread) = workspace(store, "evidence").await;
    let (other_ws, other_worker, other_thread) = workspace(store, "elsewhere").await;
    let version = |t| async move { store.thread_version(t).await.expect("version") };
    assert_eq!(version(thread).await, 0, "nothing written yet");

    // Each content write moves the version by one.
    let msg = store
        .post_message(NewMessage {
            thread_id: thread,
            author_id: worker,
            body: "first draft".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .expect("post");
    assert_eq!(version(thread).await, 1, "a post");
    store
        .edit_message(
            msg.id,
            worker,
            EditMessage {
                body: "second draft".into(),
                metadata: json!({}),
                content: None,
            },
        )
        .await
        .expect("edit");
    assert_eq!(version(thread).await, 2, "an edit");
    store
        .set_thread_result(thread, worker, &json!({"status": "done"}))
        .await
        .expect("result");
    assert_eq!(version(thread).await, 3, "a result");
    store
        .set_thread_result(thread, worker, &json!({"status": "done again"}))
        .await
        .expect("result again");
    assert_eq!(version(thread).await, 4, "a replaced result");
    store
        .set_thread_title(thread, Some("renamed".into()))
        .await
        .expect("title");
    assert_eq!(version(thread).await, 5, "a new title");
    store.tombstone_message(msg.id).await.expect("tombstone");
    assert_eq!(version(thread).await, 6, "a tombstone");

    // A write that changes nothing a reviewer reads leaves it alone.
    store
        .set_thread_title(thread, Some("renamed".into()))
        .await
        .expect("same title");
    store.claim_thread(thread, worker).await.expect("claim");
    assert_eq!(version(thread).await, 6, "the same title and a claim");
    assert_eq!(version(other_thread).await, 0, "another thread's writes");

    // Linking needs an artifact the thread's workspace holds.
    store
        .record_artifact_ref(ws, HELD)
        .await
        .expect("held here");
    store
        .record_artifact_ref(other_ws, ELSEWHERE)
        .await
        .expect("held elsewhere");
    assert!(
        matches!(
            store.link_thread_artifact(thread, ELSEWHERE, worker).await,
            Err(StoreError::NotFound)
        ),
        "another workspace's artifact cannot be linked by its hash"
    );
    let (link, new) = store
        .link_thread_artifact(thread, HELD, worker)
        .await
        .expect("link");
    assert!(new);
    assert_eq!((link.thread_id, link.sha256.as_str()), (thread, HELD));
    assert_eq!(version(thread).await, 7, "a link");
    let (_, again) = store
        .link_thread_artifact(thread, HELD, worker)
        .await
        .expect("link again");
    assert!(!again);
    assert_eq!(version(thread).await, 7, "linking twice changes nothing");
    assert_eq!(
        store.list_thread_artifacts(thread).await.expect("list"),
        vec![link]
    );
    assert!(store
        .unlink_thread_artifact(thread, HELD)
        .await
        .expect("unlink"));
    assert!(!store
        .unlink_thread_artifact(thread, HELD)
        .await
        .expect("unlink again"));
    assert_eq!(version(thread).await, 8, "an unlink, once");

    assert!(matches!(
        store.thread_version(ThreadId(uuid::Uuid::now_v7())).await,
        Err(StoreError::NotFound)
    ));

    // Erasing the artifact from this workspace takes its links here with it,
    // and leaves another workspace's link to the same bytes alone.
    store
        .link_thread_artifact(thread, HELD, worker)
        .await
        .expect("relink");
    store
        .record_artifact_ref(other_ws, HELD)
        .await
        .expect("held there too");
    store
        .link_thread_artifact(other_thread, HELD, other_worker)
        .await
        .expect("linked there");
    store
        .erase_artifact_audited(ws, HELD, Box::new(erase_audit))
        .await
        .expect("erase the artifact here");
    assert!(
        store
            .list_thread_artifacts(thread)
            .await
            .expect("list")
            .is_empty(),
        "an erased artifact is no longer evidence here"
    );
    assert_eq!(version(thread).await, 10, "the relink and the erase");
    assert_eq!(
        store
            .list_thread_artifacts(other_thread)
            .await
            .expect("list there")
            .len(),
        1,
        "another workspace keeps its link"
    );

    // The triggers do not stand in the way of erasing a workspace.
    store.erase_workspace(ws).await.expect("erase");
    assert!(matches!(
        store.thread_version(thread).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn every_content_write_moves_the_thread_version_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    run_suite(&SqliteStore::for_tests(pool)).await;
}

#[tokio::test]
async fn every_content_write_moves_the_thread_version_on_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}
