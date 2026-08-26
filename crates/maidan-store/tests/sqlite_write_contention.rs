//! Reproduction harness for the SQLite first-write "database is locked" finding
//! (Cluster 277). Mirrors the server's file-backed pool: multiple connections,
//! WAL, per-connection 5 s busy_timeout. Hammers concurrent read-modify-write
//! transactions (the shape the store's `*_with_event` methods use: SELECT scope,
//! then INSERT/UPDATE, in one deferred transaction) and counts lock failures.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;
use std::sync::Arc;

fn wal_pool_options(busy_timeout_ms: u64) -> SqlitePoolOptions {
    SqlitePoolOptions::new().after_connect(move |conn, _| {
        Box::pin(async move {
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *conn)
                .await?;
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&mut *conn)
                .await?;
            sqlx::query(&format!("PRAGMA busy_timeout = {busy_timeout_ms}"))
                .execute(&mut *conn)
                .await?;
            Ok(())
        })
    })
}

async fn run_contention(max_connections: u32) -> usize {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("contention.db");
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let pool = wal_pool_options(5000)
        .max_connections(max_connections)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO t (id, n) VALUES (1, 0)")
        .execute(&pool)
        .await
        .unwrap();

    let pool = Arc::new(pool);
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let pool = pool.clone();
        set.spawn(async move {
            let mut locked = 0usize;
            for _ in 0..25 {
                // read-modify-write in one deferred transaction (the deadlock-prone
                // shape the store's `*_with_event` methods use: SELECT scope, then write).
                let mut tx = pool.begin().await.unwrap();
                let cur: i64 = match sqlx::query("SELECT n FROM t WHERE id = 1")
                    .fetch_one(&mut *tx)
                    .await
                {
                    Ok(row) => row.get::<i64, _>("n"),
                    Err(e) => {
                        if e.to_string().to_lowercase().contains("database is locked") {
                            locked += 1;
                        }
                        continue;
                    }
                };
                match sqlx::query("UPDATE t SET n = ? WHERE id = 1")
                    .bind(cur + 1)
                    .execute(&mut *tx)
                    .await
                {
                    Ok(_) => {
                        if let Err(e) = tx.commit().await {
                            if e.to_string().to_lowercase().contains("database is locked") {
                                locked += 1;
                            }
                        }
                    }
                    Err(e) => {
                        if e.to_string().to_lowercase().contains("database is locked") {
                            locked += 1;
                        }
                    }
                }
            }
            locked
        });
    }
    let mut total_locked = 0;
    while let Some(res) = set.join_next().await {
        total_locked += res.unwrap();
    }
    total_locked
}

/// The shipped default (`DEFAULT_SQLITE_MAX_CONNECTIONS`) survives heavy concurrent
/// read-modify-write contention with zero "database is locked" failures. This guards
/// the Cluster-277 fix: if the default is ever bumped above 1, the deferred-write
/// deadlock returns and this test fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_sqlite_pool_survives_write_contention() {
    let locked = run_contention(maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS).await;
    assert_eq!(
        locked,
        0,
        "the default SQLite pool ({} connection[s]) hit {locked} 'database is locked' \
         failures under contention; a multi-connection deferred-write pool deadlocks",
        maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS
    );
}

/// Documents the bug the fix addresses: a multi-connection SQLite pool with deferred
/// `BEGIN` transactions deadlocks concurrent read-modify-write under WAL + busy_timeout.
/// `#[ignore]`d because it asserts a racy failure (the point is that it fails a lot).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "reproduction of the deferred-write deadlock; run with --ignored"]
async fn multi_connection_pool_deadlocks_under_contention() {
    let locked = run_contention(8).await;
    eprintln!("8-connection deferred pool: {locked} 'database is locked' failures / 400 writes");
    assert!(
        locked > 0,
        "expected the multi-connection deferred-write pool to deadlock"
    );
}
