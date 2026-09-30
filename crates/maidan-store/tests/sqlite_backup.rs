//! A SQLite backup is `VACUUM INTO` (scripts/backup.sh): the snapshot of a
//! live Maidan database must be one Maidan opens and migrates as it stands.
//! `scripts/sqlite-backup-drill.sh` covers the scripts and a concurrent writer;
//! this covers the schema.

use maidan_store::{configure_sqlite_pool, prelude::*, run_sqlite_migrations};
use maidan_types::{NewChannel, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn open(path: &std::path::Path) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    configure_sqlite_pool(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn a_vacuum_into_snapshot_of_a_live_database_opens_as_maidan() {
    let dir = tempfile::tempdir().unwrap();
    let live = open(&dir.path().join("maidan.db")).await;
    run_sqlite_migrations(&live).await.unwrap();
    let store = SqliteStore::for_tests(live.clone());
    let ws = store
        .create_workspace(NewWorkspace {
            name: "backed-up".into(),
        })
        .await
        .unwrap();
    store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let snapshot = dir.path().join("maidan.sqlite");
    sqlx::query(&format!("VACUUM INTO '{}'", snapshot.display()))
        .execute(&live)
        .await
        .unwrap();
    let live_version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM maidan_migrations")
        .fetch_one(&live)
        .await
        .unwrap();

    let restored = open(&snapshot).await;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&restored)
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    let dangling: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&restored)
        .await
        .unwrap();
    assert!(dangling.is_empty(), "foreign keys dangle: {dangling:?}");
    let restored_version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM maidan_migrations")
        .fetch_one(&restored)
        .await
        .unwrap();
    assert_eq!(
        restored_version, live_version,
        "the snapshot keeps its migration record"
    );
    run_sqlite_migrations(&restored)
        .await
        .expect("a restored database migrates as it stands");

    let restored_store = SqliteStore::for_tests(restored);
    assert_eq!(
        restored_store.get_workspace(ws.id).await.unwrap().name,
        "backed-up"
    );
    let channels = restored_store.list_channels(ws.id).await.unwrap();
    assert!(channels.iter().any(|c| c.name == "general"));
}
