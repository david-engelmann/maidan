//! Migration 0136: an artifact is erased, never soft-deleted. The unused
//! `tombstoned_at` column on `maidan_artifacts` is dropped. A value tests
//! once wrote there does not delete the row.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::for_tests(pool)
}

async fn column_present(store: &SqliteStore) -> bool {
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('maidan_artifacts')")
            .fetch_all(store.pool())
            .await
            .unwrap();
    names.iter().any(|name| name == "tombstoned_at")
}

fn erase_audit(erasure: &ArtifactErasure) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(erasure.workspace_id),
        actor_id: None,
        action: "artifact.erase".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(erasure.workspace_id.0),
        metadata: serde_json::json!({ "last_reference": erasure.last_reference }),
    }
}

#[tokio::test]
async fn migration_drops_tombstoned_at_and_a_stamped_row_stays_until_erased() {
    let store = spawn().await;
    assert!(
        !column_present(&store).await,
        "a fresh database has no artifact tombstone column"
    );

    // The pre-0136 shape. Only tests ever wrote the column.
    sqlx::query("ALTER TABLE maidan_artifacts ADD COLUMN tombstoned_at TEXT")
        .execute(store.pool())
        .await
        .unwrap();
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "artifacts".into(),
        })
        .await
        .unwrap();
    let sha = "ab".repeat(32);
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.clone(),
                size_bytes: 4,
                mime_type: Some("text/plain".into()),
                filename: None,
                kind: ArtifactKind::Attachment,
                uploaded_by: None,
            },
            Some(workspace.id),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE maidan_artifacts SET tombstoned_at = ? WHERE sha256 = ?")
        .bind(chrono::Utc::now())
        .bind(&sha)
        .execute(store.pool())
        .await
        .unwrap();

    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/0136_artifact_erase.sql"
    ))
    .execute(store.pool())
    .await
    .unwrap();

    assert!(!column_present(&store).await);
    let still = store
        .get_artifact_for_workspace(workspace.id, &sha)
        .await
        .unwrap();
    assert_eq!(
        still.sha256, sha,
        "dropping the column does not erase the bytes"
    );

    let erasure = store
        .erase_artifact_audited(workspace.id, &sha, Box::new(erase_audit))
        .await
        .unwrap();
    assert!(erasure.last_reference);
    assert!(
        matches!(
            store.get_artifact_by_sha(&sha).await,
            Err(StoreError::NotFound)
        ),
        "the last reference takes the row with it"
    );
    assert!(matches!(
        store.get_artifact_for_workspace(workspace.id, &sha).await,
        Err(StoreError::NotFound)
    ));
}
