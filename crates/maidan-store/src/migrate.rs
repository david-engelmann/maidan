use sqlx::{PgPool, SqlitePool};

use crate::error::StoreError;
use std::collections::HashSet;

/// One registered migration: `migrations/<dialect>/<file>`, applied as
/// `version`. `build.rs` generates the two lists from the directories, so a
/// file on disk is a registered migration and there is no list to edit.
struct Migration {
    version: i64,
    file: &'static str,
    sql: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/migrations.rs"));

/// The one Rust step between two migrations: the event hash chain is
/// backfilled once its columns exist, before the next version relies on it.
const POSTGRES_CHAIN_BACKFILL_AFTER: i64 = 91;
const SQLITE_CHAIN_BACKFILL_AFTER: i64 = 90;

// 0162 is Postgres only: SQLite keeps the event log in one table.

/// Session advisory-lock key guarding boot-time migrations. Any constant works
/// as long as it is stable across replicas; this is the ASCII for `"migr"`,
/// chosen to be readable and unlikely to collide with application locks.
const MIGRATION_LOCK_KEY: i64 = 0x6D69_6772_i64;

/// Apply all Postgres migrations to the pool, in order, idempotently.
///
/// Tracks applied migrations in a `maidan_migrations` table. Calling
/// this repeatedly is safe; a migration only runs the first time it is
/// seen.
///
/// Boot-time migration is serialized across replicas by a Postgres session
/// advisory lock: when several replicas start against a fresh database they
/// would otherwise race on non-transactional DDL (e.g. concurrent
/// `CREATE EXTENSION`, which fails with a `pg_extension` unique violation). The
/// first replica holds the lock and migrates; the rest block, then observe the
/// migrations already applied and no-op.
pub async fn run_postgres_migrations(pool: &PgPool) -> Result<(), StoreError> {
    let mut lock_conn = pool.acquire().await?;
    // Exempt the migration session from the configured statement and lock
    // timeouts: the advisory-lock wait below can legitimately block while
    // another replica migrates, DDL must not be capped, and DDL may have to wait
    // on a lock a running replica holds during a rolling deploy.
    sqlx::query("SET statement_timeout = 0")
        .execute(&mut *lock_conn)
        .await?;
    sqlx::query("SET lock_timeout = 0")
        .execute(&mut *lock_conn)
        .await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await?;

    let result = apply_all_postgres(pool).await;

    // Release explicitly; dropping the connection would also release it, but an
    // explicit unlock returns the slot to the pool promptly.
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await;
    result
}

async fn apply_all_postgres(pool: &PgPool) -> Result<(), StoreError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS maidan_migrations (
            version BIGINT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;
    // One read of what is applied, rather than a query per migration: a boot
    // against a database across a network paid a round trip for each of them.
    // The advisory lock is held, so nothing applies a migration meanwhile.
    let applied = applied_versions_postgres(pool).await?;

    // 125 and 133 are unused; 129 is #1142's session-from-token migration.
    for migration in POSTGRES_MIGRATIONS {
        tracing::trace!(
            version = migration.version,
            file = migration.file,
            "postgres migration"
        );
        apply_postgres(pool, &applied, migration.version, migration.sql).await?;
        if migration.version == POSTGRES_CHAIN_BACKFILL_AFTER {
            crate::postgres::events::backfill_chain(pool).await?;
        }
    }
    // The partitioned tables need their months ready before the first write,
    // or rows land in DEFAULT until the first retention sweep.
    crate::postgres::partitions::maintain(pool, chrono::Utc::now()).await?;
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
    let applied = applied_versions_sqlite(pool).await?;

    // 125 and 133 are unused; 129 is #1142's session-from-token migration.
    for migration in SQLITE_MIGRATIONS {
        tracing::trace!(
            version = migration.version,
            file = migration.file,
            "sqlite migration"
        );
        apply_sqlite(pool, &applied, migration.version, migration.sql).await?;
        if migration.version == SQLITE_CHAIN_BACKFILL_AFTER {
            crate::sqlite::events::backfill_chain(pool).await?;
        }
    }
    Ok(())
}

async fn applied_versions_postgres(pool: &PgPool) -> Result<HashSet<i64>, StoreError> {
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT version FROM maidan_migrations")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
}

async fn applied_versions_sqlite(pool: &SqlitePool) -> Result<HashSet<i64>, StoreError> {
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT version FROM maidan_migrations")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
}

async fn apply_postgres(
    pool: &PgPool,
    applied: &HashSet<i64>,
    version: i64,
    sql: &str,
) -> Result<(), StoreError> {
    if applied.contains(&version) {
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

async fn apply_sqlite(
    pool: &SqlitePool,
    applied: &HashSet<i64>,
    version: i64,
    sql: &str,
) -> Result<(), StoreError> {
    if applied.contains(&version) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn on_disk(dialect: &str) -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../migrations")
            .join(dialect);
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".sql") && !n.ends_with("_down.sql"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    }

    fn check(dialect: &str, register: &[Migration], hook: i64) {
        let files: Vec<String> = register.iter().map(|m| m.file.to_string()).collect();
        assert!(files.len() > 100, "{dialect}: register has {}", files.len());
        assert_eq!(
            files,
            on_disk(dialect),
            "{dialect}: register is not the directory"
        );
        assert!(
            register.windows(2).all(|w| w[0].version < w[1].version),
            "{dialect}: versions are not strictly increasing"
        );
        for m in register {
            assert!(
                m.file.starts_with(&format!("{:04}_", m.version)),
                "{}",
                m.file
            );
            assert!(!m.sql.trim().is_empty(), "{dialect}/{} is empty", m.file);
        }
        assert!(
            register.iter().any(|m| m.version == hook),
            "{dialect}: the chain backfill runs after version {hook}, which is not registered"
        );
        for unused in [125, 133] {
            assert!(
                register.iter().all(|m| m.version != unused),
                "{unused} is used"
            );
        }
    }

    #[test]
    fn the_postgres_register_is_the_directory() {
        check(
            "postgres",
            POSTGRES_MIGRATIONS,
            POSTGRES_CHAIN_BACKFILL_AFTER,
        );
    }

    #[test]
    fn the_sqlite_register_is_the_directory() {
        check("sqlite", SQLITE_MIGRATIONS, SQLITE_CHAIN_BACKFILL_AFTER);
    }
}
