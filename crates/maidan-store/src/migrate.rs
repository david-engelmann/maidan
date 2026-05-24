use sqlx::{PgPool, SqlitePool};

use crate::error::StoreError;

const POSTGRES_UP_V1: &str = include_str!("../../../migrations/postgres/0001_core_up.sql");
const POSTGRES_UP_V2: &str = include_str!("../../../migrations/postgres/0002_search.sql");
const POSTGRES_UP_V3: &str = include_str!("../../../migrations/postgres/0003_embeddings.sql");
const POSTGRES_UP_V4: &str = include_str!("../../../migrations/postgres/0004_thread_fsm.sql");
const POSTGRES_UP_V5: &str = include_str!("../../../migrations/postgres/0005_parent_threads.sql");
const POSTGRES_UP_V6: &str = include_str!("../../../migrations/postgres/0006_event_log.sql");
const POSTGRES_UP_V7: &str = include_str!("../../../migrations/postgres/0007_artifact_kinds.sql");
const POSTGRES_UP_V8: &str = include_str!("../../../migrations/postgres/0008_api_tokens.sql");
const POSTGRES_UP_V9: &str = include_str!("../../../migrations/postgres/0009_federation_peers.sql");
const POSTGRES_UP_V10: &str =
    include_str!("../../../migrations/postgres/0010_peer_outbound_secret.sql");
const POSTGRES_UP_V11: &str =
    include_str!("../../../migrations/postgres/0011_peer_remote_workspace.sql");
const SQLITE_UP_V1: &str = include_str!("../../../migrations/sqlite/0001_core_up.sql");
const SQLITE_UP_V2: &str = include_str!("../../../migrations/sqlite/0002_search.sql");
const SQLITE_UP_V4: &str = include_str!("../../../migrations/sqlite/0004_thread_fsm.sql");
const SQLITE_UP_V5: &str = include_str!("../../../migrations/sqlite/0005_parent_threads.sql");
const SQLITE_UP_V6: &str = include_str!("../../../migrations/sqlite/0006_event_log.sql");
const SQLITE_UP_V7: &str = include_str!("../../../migrations/sqlite/0007_artifact_kinds.sql");
const SQLITE_UP_V8: &str = include_str!("../../../migrations/sqlite/0008_api_tokens.sql");
const SQLITE_UP_V9: &str = include_str!("../../../migrations/sqlite/0009_federation_peers.sql");
const SQLITE_UP_V10: &str =
    include_str!("../../../migrations/sqlite/0010_peer_outbound_secret.sql");
const SQLITE_UP_V11: &str =
    include_str!("../../../migrations/sqlite/0011_peer_remote_workspace.sql");

/// Apply all Postgres migrations to the pool, in order, idempotently.
///
/// Tracks applied migrations in a `maidan_migrations` table. Calling
/// this repeatedly is safe; a migration only runs the first time it is
/// seen.
pub async fn run_postgres_migrations(pool: &PgPool) -> Result<(), StoreError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS maidan_migrations (
            version BIGINT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;

    apply_postgres(pool, 1, POSTGRES_UP_V1).await?;
    apply_postgres(pool, 2, POSTGRES_UP_V2).await?;
    apply_postgres(pool, 3, POSTGRES_UP_V3).await?;
    apply_postgres(pool, 4, POSTGRES_UP_V4).await?;
    apply_postgres(pool, 5, POSTGRES_UP_V5).await?;
    apply_postgres(pool, 6, POSTGRES_UP_V6).await?;
    apply_postgres(pool, 7, POSTGRES_UP_V7).await?;
    apply_postgres(pool, 8, POSTGRES_UP_V8).await?;
    apply_postgres(pool, 9, POSTGRES_UP_V9).await?;
    apply_postgres(pool, 10, POSTGRES_UP_V10).await?;
    apply_postgres(pool, 11, POSTGRES_UP_V11).await?;
    Ok(())
}

/// Apply all SQLite migrations to the pool, idempotently.
pub async fn run_sqlite_migrations(pool: &SqlitePool) -> Result<(), StoreError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS maidan_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await?;

    apply_sqlite(pool, 1, SQLITE_UP_V1).await?;
    apply_sqlite(pool, 2, SQLITE_UP_V2).await?;
    apply_sqlite(pool, 4, SQLITE_UP_V4).await?;
    apply_sqlite(pool, 5, SQLITE_UP_V5).await?;
    apply_sqlite(pool, 6, SQLITE_UP_V6).await?;
    apply_sqlite(pool, 7, SQLITE_UP_V7).await?;
    apply_sqlite(pool, 8, SQLITE_UP_V8).await?;
    apply_sqlite(pool, 9, SQLITE_UP_V9).await?;
    apply_sqlite(pool, 10, SQLITE_UP_V10).await?;
    apply_sqlite(pool, 11, SQLITE_UP_V11).await?;
    Ok(())
}

async fn apply_postgres(pool: &PgPool, version: i64, sql: &str) -> Result<(), StoreError> {
    let already: Option<(i64,)> =
        sqlx::query_as("SELECT version FROM maidan_migrations WHERE version = $1")
            .bind(version)
            .fetch_optional(pool)
            .await?;
    if already.is_some() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    sqlx::raw_sql(sql).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO maidan_migrations (version) VALUES ($1)")
        .bind(version)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(version, "applied postgres migration");
    Ok(())
}

async fn apply_sqlite(pool: &SqlitePool, version: i64, sql: &str) -> Result<(), StoreError> {
    let already: Option<(i64,)> =
        sqlx::query_as("SELECT version FROM maidan_migrations WHERE version = ?")
            .bind(version)
            .fetch_optional(pool)
            .await?;
    if already.is_some() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    sqlx::raw_sql(sql).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO maidan_migrations (version) VALUES (?)")
        .bind(version)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(version, "applied sqlite migration");
    Ok(())
}
