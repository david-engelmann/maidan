//! Migration 0007: artifact kind CHECK constraint.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{ArtifactKind, NewArtifact};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn sqlite_migration_0007_rejects_unknown_artifact_kind() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let err = sqlx::query(
        "INSERT INTO maidan_artifacts (id, sha256, size_bytes, kind, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind("deadbeef".repeat(8))
    .bind(1_i64)
    .bind("not_a_kind")
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect_err("unknown kind must fail CHECK");

    assert!(
        err.to_string().contains("CHECK"),
        "expected CHECK constraint failure, got: {err}"
    );
}

#[tokio::test]
async fn sqlite_upsert_artifact_roundtrips_all_kinds() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::new(pool);

    for (i, kind) in [
        ArtifactKind::Screenshot,
        ArtifactKind::Recording,
        ArtifactKind::Transcript,
        ArtifactKind::CodeDump,
        ArtifactKind::Attachment,
    ]
    .into_iter()
    .enumerate()
    {
        let sha = format!("{:064x}", i + 1);
        let artifact = store
            .upsert_artifact(NewArtifact {
                sha256: sha.clone(),
                size_bytes: i as i64 + 1,
                mime_type: None,
                kind,
                uploaded_by: None,
            })
            .await
            .unwrap_or_else(|e| panic!("upsert {kind:?}: {e}"));
        assert_eq!(artifact.kind, kind);
        let fetched = store.get_artifact_by_sha(&sha).await.expect("get");
        assert_eq!(fetched.kind, kind);
    }
}
