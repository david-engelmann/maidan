use chrono::{DateTime, Utc};
use maidan_types::{BudgetLimits, ThreadBudget, ThreadId, UsageDelta};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
     used_tokens, used_usd_micros, used_turns, created_at, updated_at";

/// Set (upsert) a thread's budget maxima (Cluster 358, T1/T5). Accumulated usage
/// is preserved. Timestamps are bound as rfc3339 (not the `datetime('now')`
/// default) so they read back as `DateTime<Utc>` cleanly.
pub async fn set_budget(
    pool: &SqlitePool,
    thread_id: ThreadId,
    limits: BudgetLimits,
) -> Result<ThreadBudget, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_budgets
             (thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             max_tokens = excluded.max_tokens,
             max_usd_micros = excluded.max_usd_micros,
             max_turns = excluded.max_turns,
             max_wall_secs = excluded.max_wall_secs,
             updated_at = excluded.updated_at
         RETURNING thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
             used_tokens, used_usd_micros, used_turns, created_at, updated_at",
    )
    .bind(thread_id.0)
    .bind(limits.max_tokens)
    .bind(limits.max_usd_micros)
    .bind(limits.max_turns)
    .bind(limits.max_wall_secs)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_budget(&row))
}

pub async fn get_budget(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadBudget>, StoreError> {
    let sql = format!("SELECT {COLS} FROM maidan_thread_budgets WHERE thread_id = ?");
    let row = sqlx::query(&sql)
        .bind(thread_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row.as_ref().map(row_to_budget))
}

/// Accumulate reported usage onto a thread's budget (Cluster 358), creating the
/// row (with no maxima) when the thread has no budget yet. Returns the new totals.
pub async fn add_usage(
    pool: &SqlitePool,
    thread_id: ThreadId,
    delta: UsageDelta,
) -> Result<ThreadBudget, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_budgets
             (thread_id, used_tokens, used_usd_micros, used_turns, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             used_tokens = maidan_thread_budgets.used_tokens + excluded.used_tokens,
             used_usd_micros = maidan_thread_budgets.used_usd_micros + excluded.used_usd_micros,
             used_turns = maidan_thread_budgets.used_turns + excluded.used_turns,
             updated_at = excluded.updated_at
         RETURNING thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
             used_tokens, used_usd_micros, used_turns, created_at, updated_at",
    )
    .bind(thread_id.0)
    .bind(delta.tokens)
    .bind(delta.usd_micros)
    .bind(delta.turns)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_budget(&row))
}

fn row_to_budget(row: &sqlx::sqlite::SqliteRow) -> ThreadBudget {
    ThreadBudget {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        max_tokens: row.get::<Option<i64>, _>("max_tokens"),
        max_usd_micros: row.get::<Option<i64>, _>("max_usd_micros"),
        max_turns: row.get::<Option<i64>, _>("max_turns"),
        max_wall_secs: row.get::<Option<i64>, _>("max_wall_secs"),
        used_tokens: row.get::<i64, _>("used_tokens"),
        used_usd_micros: row.get::<i64, _>("used_usd_micros"),
        used_turns: row.get::<i64, _>("used_turns"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}
