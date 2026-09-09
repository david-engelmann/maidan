use chrono::{DateTime, Utc};
use maidan_types::{
    BudgetLimits, ChannelId, Event, MemberId, NewDlqEntry, StoredEvent, ThreadBudget, ThreadId,
    UsageDelta, UsageReport, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::{dlq, events, threads};
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

/// Accumulate usage on a caller-supplied tx (Cluster 358.3) — the in-tx core of
/// [`add_usage`], used by [`report_usage`] so accumulate + enforce are atomic.
async fn add_usage_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
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
    .fetch_one(&mut **tx)
    .await?;
    Ok(row_to_budget(&row))
}

/// Report usage and enforce the budget (Cluster 358.3) — the "stop the run" path.
/// See the Postgres twin. Accumulate + release + `ClaimFailed` + DLQ in one tx.
pub async fn report_usage(
    pool: &SqlitePool,
    thread_id: ThreadId,
    delta: UsageDelta,
) -> Result<(UsageReport, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let budget = add_usage_in_tx(&mut tx, thread_id, delta).await?;

    let ctx = sqlx::query(
        "SELECT t.assignee_id, t.work_started_at, t.channel_id, c.workspace_id
         FROM maidan_threads t JOIN maidan_channels c ON c.id = t.channel_id
         WHERE t.id = ? AND t.tombstoned_at IS NULL",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;

    let assignee = ctx.get::<Option<Uuid>, _>("assignee_id").map(MemberId);
    let work_started_at = ctx.get::<Option<DateTime<Utc>>, _>("work_started_at");
    let channel_id = ChannelId(ctx.get::<Uuid, _>("channel_id"));
    let workspace_id = WorkspaceId(ctx.get::<Uuid, _>("workspace_id"));
    let wall = work_started_at.map(|w| (Utc::now() - w).num_seconds());

    let (stopped, reason, stored) = match (budget.exceeded(wall), assignee) {
        (Some(reason), Some(member)) => {
            let now = Utc::now().to_rfc3339();
            let row = sqlx::query(
                "UPDATE maidan_threads
                 SET assignee_id = NULL, claim_lease_id = NULL, work_started_at = NULL, updated_at = ?
                 WHERE id = ?
                 RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
            )
            .bind(&now)
            .bind(thread_id.0)
            .fetch_one(&mut *tx)
            .await?;
            let thread = threads::row_to_thread(&row)?;
            let reason_str = reason.as_str().to_string();
            let event = Event::ClaimFailed {
                occurred_at: Utc::now(),
                workspace_id,
                channel_id,
                thread_id,
                member_id: member,
                reason: reason_str.clone(),
                thread,
            };
            let stored = events::append_in_tx(&mut tx, &event).await?;
            dlq::record_in_tx(
                &mut tx,
                &NewDlqEntry {
                    workspace_id,
                    channel_id,
                    thread_id,
                    member_id: member,
                    reason: reason_str.clone(),
                    used_tokens: budget.used_tokens,
                    used_usd_micros: budget.used_usd_micros,
                    used_turns: budget.used_turns,
                },
            )
            .await?;
            (true, Some(reason_str), Some(stored))
        }
        _ => (false, None, None),
    };
    tx.commit().await?;
    Ok((
        UsageReport {
            budget,
            stopped,
            reason,
        },
        stored,
    ))
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
