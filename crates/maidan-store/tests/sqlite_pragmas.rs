//! SQLite PRAGMA configuration (Track T.6).

use maidan_store::configure_sqlite_pool;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn configure_pool_enables_wal_on_file_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("maidan.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());

    let pool = SqlitePoolOptions::new()
        .connect(&url)
        .await
        .expect("connect");
    configure_sqlite_pool(&pool).await.expect("configure");

    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .expect("foreign_keys");
    assert_eq!(fk, 1);

    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .expect("journal_mode");
    assert_eq!(journal.to_lowercase(), "wal");
}
