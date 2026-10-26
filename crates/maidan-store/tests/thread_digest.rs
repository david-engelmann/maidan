//! A thread's digest on both backends: the newest linked artifact of kind
//! `digest`, ignoring other kinds and bytes the workspace does not hold.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const NOTES: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const FIRST: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const SECOND: &str = "3333333333333333333333333333333333333333333333333333333333333333";

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
            created_by: None,
        })
        .await
        .expect("thread")
        .id;
    (ws, member, thread)
}

async fn upload(store: &dyn Store, ws: WorkspaceId, sha: &str, kind: ArtifactKind) {
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.into(),
                size_bytes: 42,
                mime_type: Some(kind.default_mime().into()),
                filename: None,
                kind,
                uploaded_by: None,
            },
            Some(ws),
        )
        .await
        .expect("upload");
}

async fn run_suite(store: &dyn Store) {
    let (ws, worker, thread) = workspace(store, "digests").await;
    let (other_ws, _, _) = workspace(store, "elsewhere").await;
    let current = |w, t| async move {
        maidan_store::thread_digest::current(store, w, t)
            .await
            .expect("digest")
    };
    assert_eq!(current(ws, thread).await, None, "nothing linked");

    upload(store, ws, NOTES, ArtifactKind::Transcript).await;
    store
        .link_thread_artifact(thread, NOTES, worker)
        .await
        .expect("link");
    assert_eq!(
        current(ws, thread).await,
        None,
        "a transcript is not a digest"
    );

    upload(store, ws, FIRST, ArtifactKind::Digest).await;
    store
        .link_thread_artifact(thread, FIRST, worker)
        .await
        .expect("link");
    let first = current(ws, thread).await.expect("a digest");
    assert_eq!(first.sha256, FIRST);
    assert_eq!(first.uri, format!("maidan://artifacts/{FIRST}"));
    assert_eq!(first.mime_type.as_deref(), Some("text/markdown"));
    assert_eq!(first.linked_by, worker);

    tokio::time::sleep(Duration::from_millis(5)).await;
    upload(store, ws, SECOND, ArtifactKind::Digest).await;
    store
        .link_thread_artifact(thread, SECOND, worker)
        .await
        .expect("link");
    assert_eq!(
        current(ws, thread).await.expect("a digest").sha256,
        SECOND,
        "the newest link wins"
    );

    store
        .unlink_thread_artifact(thread, SECOND)
        .await
        .expect("unlink");
    assert_eq!(
        current(ws, thread).await.expect("a digest").sha256,
        FIRST,
        "unlinking falls back"
    );

    assert_eq!(
        current(other_ws, thread).await,
        None,
        "another workspace holds none of the bytes"
    );
}

#[tokio::test]
async fn the_newest_linked_digest_is_the_threads_digest_on_sqlite() {
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
async fn the_newest_linked_digest_is_the_threads_digest_on_postgres() {
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
