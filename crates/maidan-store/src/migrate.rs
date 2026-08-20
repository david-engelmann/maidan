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
const POSTGRES_UP_V12: &str = include_str!("../../../migrations/postgres/0012_oidc_sessions.sql");
const POSTGRES_UP_V13: &str = include_str!("../../../migrations/postgres/0013_outbox.sql");
const POSTGRES_UP_V14: &str =
    include_str!("../../../migrations/postgres/0014_outbox_quarantine.sql");
const POSTGRES_UP_V15: &str = include_str!("../../../migrations/postgres/0015_delivery_cursor.sql");
const POSTGRES_UP_V16: &str =
    include_str!("../../../migrations/postgres/0016_dm_conversations.sql");
const POSTGRES_UP_V17: &str = include_str!("../../../migrations/postgres/0017_inbox_cursor.sql");
const POSTGRES_UP_V18: &str = include_str!("../../../migrations/postgres/0018_reactions_pins.sql");
const POSTGRES_UP_V19: &str = include_str!("../../../migrations/postgres/0019_message_edits.sql");
const POSTGRES_UP_V20: &str =
    include_str!("../../../migrations/postgres/0020_embedding_models.sql");
const POSTGRES_UP_V21: &str = include_str!("../../../migrations/postgres/0021_webhooks.sql");
const POSTGRES_UP_V22: &str = include_str!("../../../migrations/postgres/0022_slash_commands.sql");
const POSTGRES_UP_V23: &str = include_str!("../../../migrations/postgres/0023_fsm_hooks.sql");
const POSTGRES_UP_V24: &str = include_str!("../../../migrations/postgres/0024_token_quotas.sql");
const POSTGRES_UP_V25: &str = include_str!("../../../migrations/postgres/0025_agent_apps.sql");
const POSTGRES_UP_V26: &str =
    include_str!("../../../migrations/postgres/0026_automation_deliveries.sql");
const POSTGRES_UP_V27: &str =
    include_str!("../../../migrations/postgres/0027_a2a_push_and_tasks.sql");
const POSTGRES_UP_V28: &str =
    include_str!("../../../migrations/postgres/0028_group_dm_and_mention_webhook.sql");
const POSTGRES_UP_V29: &str = include_str!("../../../migrations/postgres/0029_oauth_codes.sql");
const POSTGRES_UP_V30: &str = include_str!("../../../migrations/postgres/0030_reindex_jobs.sql");
const POSTGRES_UP_V31: &str =
    include_str!("../../../migrations/postgres/0031_events_inserted_at.sql");
const POSTGRES_UP_V32: &str = include_str!("../../../migrations/postgres/0032_channel_members.sql");
const POSTGRES_UP_V33: &str =
    include_str!("../../../migrations/postgres/0033_threads_assignee.sql");
const POSTGRES_UP_V34: &str = include_str!("../../../migrations/postgres/0034_message_content.sql");
const POSTGRES_UP_V35: &str =
    include_str!("../../../migrations/postgres/0035_thread_claim_lease.sql");
const POSTGRES_UP_V36: &str =
    include_str!("../../../migrations/postgres/0036_artifact_workspace_refs.sql");
const POSTGRES_UP_V37: &str =
    include_str!("../../../migrations/postgres/0037_thread_dependencies.sql");
const POSTGRES_UP_V38: &str = include_str!("../../../migrations/postgres/0038_task_schedules.sql");
const POSTGRES_UP_V39: &str = include_str!("../../../migrations/postgres/0039_member_skills.sql");
const POSTGRES_UP_V40: &str =
    include_str!("../../../migrations/postgres/0040_thread_required_skills.sql");
const POSTGRES_UP_V41: &str = include_str!("../../../migrations/postgres/0041_thread_results.sql");
const POSTGRES_UP_V42: &str = include_str!("../../../migrations/postgres/0042_notifications.sql");
const POSTGRES_UP_V43: &str =
    include_str!("../../../migrations/postgres/0043_notification_dedup.sql");
const POSTGRES_UP_V44: &str =
    include_str!("../../../migrations/postgres/0044_notification_prefs.sql");
const POSTGRES_UP_V45: &str = include_str!("../../../migrations/postgres/0045_follows.sql");
const POSTGRES_UP_V46: &str = include_str!("../../../migrations/postgres/0046_member_emails.sql");
const POSTGRES_UP_V47: &str =
    include_str!("../../../migrations/postgres/0047_member_last_seen.sql");
const SQLITE_UP_V1: &str = include_str!("../../../migrations/sqlite/0001_core_up.sql");
const SQLITE_UP_V2: &str = include_str!("../../../migrations/sqlite/0002_search.sql");
const SQLITE_UP_V3: &str = include_str!("../../../migrations/sqlite/0003_embeddings.sql");
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
const SQLITE_UP_V12: &str = include_str!("../../../migrations/sqlite/0012_oidc_sessions.sql");
const SQLITE_UP_V13: &str = include_str!("../../../migrations/sqlite/0013_outbox.sql");
const SQLITE_UP_V14: &str = include_str!("../../../migrations/sqlite/0014_dm_conversations.sql");
const SQLITE_UP_V15: &str = include_str!("../../../migrations/sqlite/0015_inbox_cursor.sql");
const SQLITE_UP_V16: &str = include_str!("../../../migrations/sqlite/0016_reactions_pins.sql");
const SQLITE_UP_V17: &str = include_str!("../../../migrations/sqlite/0017_message_edits.sql");
const SQLITE_UP_V18: &str = include_str!("../../../migrations/sqlite/0018_embedding_models.sql");
const SQLITE_UP_V19: &str = include_str!("../../../migrations/sqlite/0019_webhooks.sql");
const SQLITE_UP_V20: &str = include_str!("../../../migrations/sqlite/0020_slash_commands.sql");
const SQLITE_UP_V21: &str = include_str!("../../../migrations/sqlite/0021_fsm_hooks.sql");
const SQLITE_UP_V22: &str = include_str!("../../../migrations/sqlite/0022_token_quotas.sql");
const SQLITE_UP_V23: &str = include_str!("../../../migrations/sqlite/0023_delivery_cursor.sql");
const SQLITE_UP_V24: &str = include_str!("../../../migrations/sqlite/0024_agent_apps.sql");
const SQLITE_UP_V25: &str =
    include_str!("../../../migrations/sqlite/0025_automation_deliveries.sql");
const SQLITE_UP_V26: &str = include_str!("../../../migrations/sqlite/0026_a2a_push_and_tasks.sql");
const SQLITE_UP_V27: &str =
    include_str!("../../../migrations/sqlite/0027_group_dm_and_mention_webhook.sql");
const SQLITE_UP_V28: &str = include_str!("../../../migrations/sqlite/0028_oauth_codes.sql");
const SQLITE_UP_V29: &str = include_str!("../../../migrations/sqlite/0029_reindex_jobs.sql");
const SQLITE_UP_V30: &str = include_str!("../../../migrations/sqlite/0030_events_inserted_at.sql");
const SQLITE_UP_V31: &str = include_str!("../../../migrations/sqlite/0031_channel_members.sql");
const SQLITE_UP_V32: &str = include_str!("../../../migrations/sqlite/0032_threads_assignee.sql");
const SQLITE_UP_V33: &str = include_str!("../../../migrations/sqlite/0033_message_content.sql");
const SQLITE_UP_V34: &str = include_str!("../../../migrations/sqlite/0034_thread_claim_lease.sql");
const SQLITE_UP_V35: &str =
    include_str!("../../../migrations/sqlite/0035_artifact_workspace_refs.sql");
const SQLITE_UP_V36: &str = include_str!("../../../migrations/sqlite/0036_thread_dependencies.sql");
const SQLITE_UP_V37: &str = include_str!("../../../migrations/sqlite/0037_task_schedules.sql");
const SQLITE_UP_V38: &str = include_str!("../../../migrations/sqlite/0038_member_skills.sql");
const SQLITE_UP_V39: &str =
    include_str!("../../../migrations/sqlite/0039_thread_required_skills.sql");
const SQLITE_UP_V40: &str = include_str!("../../../migrations/sqlite/0040_thread_results.sql");
const SQLITE_UP_V41: &str = include_str!("../../../migrations/sqlite/0041_notifications.sql");
const SQLITE_UP_V42: &str = include_str!("../../../migrations/sqlite/0042_notification_dedup.sql");
const SQLITE_UP_V43: &str = include_str!("../../../migrations/sqlite/0043_notification_prefs.sql");
const SQLITE_UP_V44: &str = include_str!("../../../migrations/sqlite/0044_follows.sql");
const SQLITE_UP_V45: &str = include_str!("../../../migrations/sqlite/0045_member_emails.sql");
const SQLITE_UP_V46: &str = include_str!("../../../migrations/sqlite/0046_member_last_seen.sql");

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
    // Exempt the migration session from any configured `statement_timeout`
    // (Cluster 107): the advisory-lock wait below can legitimately block while
    // another replica migrates, and DDL must not be capped.
    sqlx::query("SET statement_timeout = 0")
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
    apply_postgres(pool, 12, POSTGRES_UP_V12).await?;
    apply_postgres(pool, 13, POSTGRES_UP_V13).await?;
    apply_postgres(pool, 14, POSTGRES_UP_V14).await?;
    apply_postgres(pool, 15, POSTGRES_UP_V15).await?;
    apply_postgres(pool, 16, POSTGRES_UP_V16).await?;
    apply_postgres(pool, 17, POSTGRES_UP_V17).await?;
    apply_postgres(pool, 18, POSTGRES_UP_V18).await?;
    apply_postgres(pool, 19, POSTGRES_UP_V19).await?;
    apply_postgres(pool, 20, POSTGRES_UP_V20).await?;
    apply_postgres(pool, 21, POSTGRES_UP_V21).await?;
    apply_postgres(pool, 22, POSTGRES_UP_V22).await?;
    apply_postgres(pool, 23, POSTGRES_UP_V23).await?;
    apply_postgres(pool, 24, POSTGRES_UP_V24).await?;
    apply_postgres(pool, 25, POSTGRES_UP_V25).await?;
    apply_postgres(pool, 26, POSTGRES_UP_V26).await?;
    apply_postgres(pool, 27, POSTGRES_UP_V27).await?;
    apply_postgres(pool, 28, POSTGRES_UP_V28).await?;
    apply_postgres(pool, 29, POSTGRES_UP_V29).await?;
    apply_postgres(pool, 30, POSTGRES_UP_V30).await?;
    apply_postgres(pool, 31, POSTGRES_UP_V31).await?;
    apply_postgres(pool, 32, POSTGRES_UP_V32).await?;
    apply_postgres(pool, 33, POSTGRES_UP_V33).await?;
    apply_postgres(pool, 34, POSTGRES_UP_V34).await?;
    apply_postgres(pool, 35, POSTGRES_UP_V35).await?;
    apply_postgres(pool, 36, POSTGRES_UP_V36).await?;
    apply_postgres(pool, 37, POSTGRES_UP_V37).await?;
    apply_postgres(pool, 38, POSTGRES_UP_V38).await?;
    apply_postgres(pool, 39, POSTGRES_UP_V39).await?;
    apply_postgres(pool, 40, POSTGRES_UP_V40).await?;
    apply_postgres(pool, 41, POSTGRES_UP_V41).await?;
    apply_postgres(pool, 42, POSTGRES_UP_V42).await?;
    apply_postgres(pool, 43, POSTGRES_UP_V43).await?;
    apply_postgres(pool, 44, POSTGRES_UP_V44).await?;
    apply_postgres(pool, 45, POSTGRES_UP_V45).await?;
    apply_postgres(pool, 46, POSTGRES_UP_V46).await?;
    apply_postgres(pool, 47, POSTGRES_UP_V47).await?;
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
    apply_sqlite(pool, 3, SQLITE_UP_V3).await?;
    apply_sqlite(pool, 4, SQLITE_UP_V4).await?;
    apply_sqlite(pool, 5, SQLITE_UP_V5).await?;
    apply_sqlite(pool, 6, SQLITE_UP_V6).await?;
    apply_sqlite(pool, 7, SQLITE_UP_V7).await?;
    apply_sqlite(pool, 8, SQLITE_UP_V8).await?;
    apply_sqlite(pool, 9, SQLITE_UP_V9).await?;
    apply_sqlite(pool, 10, SQLITE_UP_V10).await?;
    apply_sqlite(pool, 11, SQLITE_UP_V11).await?;
    apply_sqlite(pool, 12, SQLITE_UP_V12).await?;
    apply_sqlite(pool, 13, SQLITE_UP_V13).await?;
    apply_sqlite(pool, 14, SQLITE_UP_V14).await?;
    apply_sqlite(pool, 15, SQLITE_UP_V15).await?;
    apply_sqlite(pool, 16, SQLITE_UP_V16).await?;
    apply_sqlite(pool, 17, SQLITE_UP_V17).await?;
    apply_sqlite(pool, 18, SQLITE_UP_V18).await?;
    apply_sqlite(pool, 19, SQLITE_UP_V19).await?;
    apply_sqlite(pool, 20, SQLITE_UP_V20).await?;
    apply_sqlite(pool, 21, SQLITE_UP_V21).await?;
    apply_sqlite(pool, 22, SQLITE_UP_V22).await?;
    apply_sqlite(pool, 23, SQLITE_UP_V23).await?;
    apply_sqlite(pool, 24, SQLITE_UP_V24).await?;
    apply_sqlite(pool, 25, SQLITE_UP_V25).await?;
    apply_sqlite(pool, 26, SQLITE_UP_V26).await?;
    apply_sqlite(pool, 27, SQLITE_UP_V27).await?;
    apply_sqlite(pool, 28, SQLITE_UP_V28).await?;
    apply_sqlite(pool, 29, SQLITE_UP_V29).await?;
    apply_sqlite(pool, 30, SQLITE_UP_V30).await?;
    apply_sqlite(pool, 31, SQLITE_UP_V31).await?;
    apply_sqlite(pool, 32, SQLITE_UP_V32).await?;
    apply_sqlite(pool, 33, SQLITE_UP_V33).await?;
    apply_sqlite(pool, 34, SQLITE_UP_V34).await?;
    apply_sqlite(pool, 35, SQLITE_UP_V35).await?;
    apply_sqlite(pool, 36, SQLITE_UP_V36).await?;
    apply_sqlite(pool, 37, SQLITE_UP_V37).await?;
    apply_sqlite(pool, 38, SQLITE_UP_V38).await?;
    apply_sqlite(pool, 39, SQLITE_UP_V39).await?;
    apply_sqlite(pool, 40, SQLITE_UP_V40).await?;
    apply_sqlite(pool, 41, SQLITE_UP_V41).await?;
    apply_sqlite(pool, 42, SQLITE_UP_V42).await?;
    apply_sqlite(pool, 43, SQLITE_UP_V43).await?;
    apply_sqlite(pool, 44, SQLITE_UP_V44).await?;
    apply_sqlite(pool, 45, SQLITE_UP_V45).await?;
    apply_sqlite(pool, 46, SQLITE_UP_V46).await?;
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
