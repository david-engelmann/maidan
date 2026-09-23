//! Purging a workspace never destroys an artifact another workspace still
//! references — on both backends.
//!
//! Artifacts are content-addressed and shared: one row per sha, owned by
//! whoever uploaded it first, with per-workspace access in
//! `maidan_artifact_refs`. Purge used to delete by `uploaded_by`, which
//! destroyed another tenant's content whenever it had uploaded the same bytes.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{ArtifactKind, MemberKind, NewArtifact, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn run_suite(store: &dyn Store) {
    let mut tenants = Vec::new();
    for name in ["first", "second"] {
        let ws = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "m".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        tenants.push((ws.id, member.id));
    }
    let (first, first_member) = tenants[0];
    let (second, _) = tenants[1];
    let artifact = |sha: &str| NewArtifact {
        sha256: sha.into(),
        size_bytes: 1,
        mime_type: None,
        kind: ArtifactKind::Attachment,
        uploaded_by: Some(first_member),
    };

    // Shared: uploaded first by `first`, referenced by both.
    let shared = "a".repeat(64);
    store.upsert_artifact(artifact(&shared)).await.unwrap();
    store.record_artifact_ref(first, &shared).await.unwrap();
    store.record_artifact_ref(second, &shared).await.unwrap();
    // Only `first`'s.
    let own = "b".repeat(64);
    store.upsert_artifact(artifact(&own)).await.unwrap();
    store.record_artifact_ref(first, &own).await.unwrap();
    // Uploaded by `first`'s member but referenced by nobody: readable by nobody.
    let stray = "c".repeat(64);
    store.upsert_artifact(artifact(&stray)).await.unwrap();

    let purged = store.purge_workspace_messages(first).await.unwrap();
    let mut removed = purged.artifact_shas.clone();
    removed.sort();
    assert_eq!(removed, vec![own.clone(), stray.clone()]);
    assert_eq!(purged.artifacts_removed, 2);
    assert!(
        store.get_artifact_by_sha(&shared).await.is_ok(),
        "the shared artifact must survive the first purge"
    );
    assert!(store.artifact_ref_exists(second, &shared).await.unwrap());
    assert!(!store.artifact_ref_exists(first, &shared).await.unwrap());
    assert!(store.get_artifact_by_sha(&own).await.is_err());

    let purged = store.purge_workspace_messages(second).await.unwrap();
    assert_eq!(purged.artifact_shas, vec![shared.clone()]);
    assert!(store.get_artifact_by_sha(&shared).await.is_err());
}

#[tokio::test]
async fn shared_artifacts_survive_a_purge_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::new(pool)).await;
}

#[tokio::test]
async fn shared_artifacts_survive_a_purge_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::new(pool)).await;
}
