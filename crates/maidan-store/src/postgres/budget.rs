use chrono::{DateTime, Utc};
use maidan_types::{
    BudgetLimits, BudgetPatch, BudgetReason, ChannelId, Event, MemberId, NewDlqEntry,
    NewUsageLedgerEntry, PayerStamp, StoredEvent, Thread, ThreadBudget, ThreadId, UsageDelta,
    UsageLedgerEntry, UsageReport, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{dlq, events, threads, usage_ledger};
use crate::error::StoreError;

const COLS: &str = "thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
     used_tokens, used_usd_micros, used_turns, used_wall_secs, created_at, updated_at, used_input_tokens, used_output_tokens, used_cache_read_tokens, used_cache_write_5m_tokens, used_cache_write_1h_tokens";

/// Set (upsert) a thread's budget maxima. Accumulated usage is preserved — this
/// touches only the `max_*` dimensions. See the SQLite twin.
pub async fn set_budget(
    pool: &PgPool,
    thread_id: ThreadId,
    limits: BudgetLimits,
) -> Result<ThreadBudget, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_budgets
             (thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (thread_id) DO UPDATE SET
             max_tokens = excluded.max_tokens,
             max_usd_micros = excluded.max_usd_micros,
             max_turns = excluded.max_turns,
             max_wall_secs = excluded.max_wall_secs,
             updated_at = now()
         RETURNING thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
             used_tokens, used_usd_micros, used_turns, used_wall_secs, created_at, updated_at, used_input_tokens, used_output_tokens, used_cache_read_tokens, used_cache_write_5m_tokens, used_cache_write_1h_tokens",
    )
    .bind(thread_id.0)
    .bind(limits.max_tokens)
    .bind(limits.max_usd_micros)
    .bind(limits.max_turns)
    .bind(limits.max_wall_secs)
    .fetch_one(pool)
    .await?;
    Ok(row_to_budget(&row))
}

pub async fn get_budget(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Option<ThreadBudget>, StoreError> {
    let sql = format!("SELECT {COLS} FROM maidan_thread_budgets WHERE thread_id = $1");
    let row = sqlx::query(&sql)
        .bind(thread_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row.as_ref().map(row_to_budget))
}

/// Accumulate reported usage onto a thread's budget, creating the row (with no
/// maxima) when the thread has no budget yet — so usage still accrues before a
/// budget is set. Returns the new totals.
pub async fn add_usage(
    pool: &PgPool,
    thread_id: ThreadId,
    delta: UsageDelta,
) -> Result<ThreadBudget, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_budgets
             (thread_id, used_tokens, used_usd_micros, used_turns)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (thread_id) DO UPDATE SET
             used_tokens = maidan_thread_budgets.used_tokens + excluded.used_tokens,
             used_usd_micros = maidan_thread_budgets.used_usd_micros + excluded.used_usd_micros,
             used_turns = maidan_thread_budgets.used_turns + excluded.used_turns,
             updated_at = now()
         RETURNING thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
             used_tokens, used_usd_micros, used_turns, used_wall_secs, created_at, updated_at, used_input_tokens, used_output_tokens, used_cache_read_tokens, used_cache_write_5m_tokens, used_cache_write_1h_tokens",
    )
    .bind(thread_id.0)
    .bind(delta.tokens)
    .bind(delta.usd_micros)
    .bind(delta.turns)
    .fetch_one(pool)
    .await?;
    Ok(row_to_budget(&row))
}

async fn add_accounted_usage_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    new: &NewUsageLedgerEntry,
) -> Result<ThreadBudget, StoreError> {
    let fresh = new.tokens.fresh().map_err(StoreError::InvalidInput)?;
    let tokens = new.tokens;
    let sql = format!(
        "INSERT INTO maidan_thread_budgets
             (thread_id, used_tokens, used_usd_micros, used_turns,
              used_input_tokens, used_output_tokens, used_cache_read_tokens,
              used_cache_write_5m_tokens, used_cache_write_1h_tokens)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (thread_id) DO UPDATE SET
             used_tokens = maidan_thread_budgets.used_tokens + excluded.used_tokens,
             used_usd_micros = maidan_thread_budgets.used_usd_micros + excluded.used_usd_micros,
             used_turns = maidan_thread_budgets.used_turns + excluded.used_turns,
             used_input_tokens = maidan_thread_budgets.used_input_tokens + excluded.used_input_tokens,
             used_output_tokens = maidan_thread_budgets.used_output_tokens + excluded.used_output_tokens,
             used_cache_read_tokens = maidan_thread_budgets.used_cache_read_tokens + excluded.used_cache_read_tokens,
             used_cache_write_5m_tokens = maidan_thread_budgets.used_cache_write_5m_tokens + excluded.used_cache_write_5m_tokens,
             used_cache_write_1h_tokens = maidan_thread_budgets.used_cache_write_1h_tokens + excluded.used_cache_write_1h_tokens,
             updated_at = now()
         RETURNING {COLS}"
    );
    let row = sqlx::query(&sql)
        .bind(new.thread_id.0)
        .bind(fresh)
        .bind(new.usd_micros)
        .bind(new.turns)
        .bind(tokens.input)
        .bind(tokens.output)
        .bind(tokens.cache_read)
        .bind(tokens.cache_write_5m)
        .bind(tokens.cache_write_1h)
        .fetch_one(&mut **tx)
        .await?;
    Ok(row_to_budget(&row))
}

/// Accumulate usage on a caller-supplied tx — the in-tx core of [`add_usage`],
/// used by [`report_usage`] so accumulate + enforce are atomic.
async fn add_usage_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    delta: UsageDelta,
) -> Result<ThreadBudget, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_budgets
             (thread_id, used_tokens, used_usd_micros, used_turns)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (thread_id) DO UPDATE SET
             used_tokens = maidan_thread_budgets.used_tokens + excluded.used_tokens,
             used_usd_micros = maidan_thread_budgets.used_usd_micros + excluded.used_usd_micros,
             used_turns = maidan_thread_budgets.used_turns + excluded.used_turns,
             updated_at = now()
         RETURNING thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
             used_tokens, used_usd_micros, used_turns, used_wall_secs, created_at, updated_at, used_input_tokens, used_output_tokens, used_cache_read_tokens, used_cache_write_5m_tokens, used_cache_write_1h_tokens",
    )
    .bind(thread_id.0)
    .bind(delta.tokens)
    .bind(delta.usd_micros)
    .bind(delta.turns)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row_to_budget(&row))
}

/// A budget stop used to drop the claim without recording the wall time that
/// stop was measured against. Persist that time before the claim is cleared,
/// so the next reader sees it in `used_wall_secs`. A stop with no working
/// clock (or a zero one) leaves the row as the usage write left it.
async fn keep_stopped_wall(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    budget: ThreadBudget,
    wall: Option<i64>,
) -> Result<ThreadBudget, StoreError> {
    let Some(secs) = wall.map(|secs| secs.max(0)).filter(|secs| *secs > 0) else {
        return Ok(budget);
    };
    Ok(charge_wall_in_tx(tx, thread_id, Some(secs))
        .await?
        .unwrap_or(budget))
}

/// Report usage and enforce the budget — the "stop the run" path. Accumulates
/// `delta`, and if the thread is now over budget AND has an active claim,
/// atomically: releases the claim, appends a `ClaimFailed` event, and records a
/// DLQ entry — all in one tx with the usage write. Returns the new totals +
/// whether the run was stopped, plus the `ClaimFailed` event to publish (the
/// route calls `publish_stored`). `NotFound` if the thread is gone.
pub async fn report_usage(
    pool: &PgPool,
    thread_id: ThreadId,
    delta: UsageDelta,
) -> Result<(UsageReport, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let budget = add_usage_in_tx(&mut tx, thread_id, delta).await?;

    let ctx = sqlx::query(
        "SELECT t.assignee_id, t.work_started_at, t.channel_id, c.workspace_id
         FROM maidan_threads t JOIN maidan_channels c ON c.id = t.channel_id
         WHERE t.id = $1 AND t.tombstoned_at IS NULL",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;

    let assignee = ctx.get::<Option<Uuid>, _>("assignee_id").map(MemberId);
    let work_started_at = ctx.get::<Option<DateTime<Utc>>, _>("work_started_at");
    let channel_id = ChannelId(ctx.get::<Uuid, _>("channel_id"));
    let workspace_id = WorkspaceId(ctx.get::<Uuid, _>("workspace_id"));
    // The working clock and this instant are both the database clock, so a
    // container whose clock differs from the host still charges what it measured.
    let clock = sqlx::query("SELECT NOW() AS ended_at")
        .fetch_one(&mut *tx)
        .await?;
    let wall = work_started_at
        .map(|started| (clock.get::<DateTime<Utc>, _>("ended_at") - started).num_seconds());

    let stop = match (budget.exceeded(wall), assignee) {
        (Some(reason), Some(member)) => Some((reason, member)),
        _ => None,
    };
    let budget = if stop.is_some() {
        keep_stopped_wall(&mut tx, thread_id, budget, wall).await?
    } else {
        budget
    };
    let (stopped, reason, stored) = if let Some((reason, member)) = stop {
        let row = sqlx::query(
            "UPDATE maidan_threads
                 SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = now()
                 WHERE id = $1
                 RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
        )
        .bind(thread_id.0)
        .fetch_one(&mut *tx)
        .await?;
        let thread = threads::row_to_thread(&row)?;
        let stored = fail_claim_in_tx(
            &mut tx,
            workspace_id,
            channel_id,
            thread,
            member,
            reason,
            &budget,
        )
        .await?;
        (true, Some(reason.as_str().to_owned()), Some(stored))
    } else {
        (false, None, None)
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

/// Claim-fenced, idempotent usage accounting. The reservation row owns the
/// economic idempotency key; every other mutation shares this transaction.
pub async fn report_accounted_usage(
    pool: &PgPool,
    new: &NewUsageLedgerEntry,
) -> Result<(UsageLedgerEntry, Vec<StoredEvent>), StoreError> {
    new.validate().map_err(StoreError::InvalidInput)?;
    let mut tx = pool.begin().await?;
    let ctx = sqlx::query(
        "SELECT t.assignee_id, t.claim_lease_id, t.work_started_at,
                t.channel_id, c.workspace_id
         FROM maidan_threads t JOIN maidan_channels c ON c.id = t.channel_id
         WHERE t.id = $1 AND t.tombstoned_at IS NULL FOR UPDATE",
    )
    .bind(new.thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let workspace_id = WorkspaceId(ctx.get::<Uuid, _>("workspace_id"));
    let channel_id = ChannelId(ctx.get::<Uuid, _>("channel_id"));

    if !usage_ledger::reserve_in_tx(&mut tx, new, workspace_id).await? {
        let existing = usage_ledger::get_in_tx(&mut tx, new.usage_report_id)
            .await?
            .ok_or_else(|| StoreError::Conflict("usage report id was not readable".into()))?;
        if existing.stamp.payer != workspace_id || !existing.matches_request(new) {
            return Err(StoreError::Conflict(
                "usage_report_id was already used for different content".into(),
            ));
        }
        tx.commit().await?;
        return Ok((existing, Vec::new()));
    }

    let assignee = ctx.get::<Option<Uuid>, _>("assignee_id").map(MemberId);
    let lease = ctx.get::<Option<Uuid>, _>("claim_lease_id");
    if assignee != Some(new.reporter) || lease != Some(new.claim_lease_id.0) {
        return Err(StoreError::Conflict(
            "usage reporter is not the active holder of this claim lease".into(),
        ));
    }

    let budget = add_accounted_usage_in_tx(&mut tx, new).await?;
    let clock = sqlx::query("SELECT NOW() AS ended_at")
        .fetch_one(&mut *tx)
        .await?;
    let wall = ctx
        .get::<Option<DateTime<Utc>>, _>("work_started_at")
        .map(|started| (clock.get::<DateTime<Utc>, _>("ended_at") - started).num_seconds());
    let stopping = budget.exceeded(wall);
    let budget = if stopping.is_some() {
        keep_stopped_wall(&mut tx, new.thread_id, budget, wall).await?
    } else {
        budget
    };
    let stamp = PayerStamp {
        payer: workspace_id,
        reporter: new.reporter,
        claim_lease_id: new.claim_lease_id,
        model: new.model.trim().to_owned(),
        tokens: new.tokens,
        usd_micros: new.usd_micros,
        price_snapshot: new.price_snapshot,
    };
    let usage = Event::UsageReported {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id: new.thread_id,
        usage_report_id: new.usage_report_id,
        stamp,
        turns: new.turns,
        budget: budget.clone(),
    };
    let usage_stored = events::append_in_tx(&mut tx, &usage).await?;

    let (stopped, reason, failed) = if let Some(reason) = stopping {
        let row = sqlx::query(
            "UPDATE maidan_threads
             SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL, work_started_at = NULL, updated_at = now()
             WHERE id = $1 AND assignee_id = $2 AND claim_lease_id = $3
             RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
        )
        .bind(new.thread_id.0)
        .bind(new.reporter.0)
        .bind(new.claim_lease_id.0)
        .fetch_one(&mut *tx)
        .await?;
        let thread = threads::row_to_thread(&row)?;
        let failed = fail_claim_in_tx(
            &mut tx,
            workspace_id,
            channel_id,
            thread,
            new.reporter,
            reason,
            &budget,
        )
        .await?;
        (true, Some(reason.as_str().to_owned()), Some(failed))
    } else {
        (false, None, None)
    };
    let entry = usage_ledger::finish_in_tx(
        &mut tx,
        new.usage_report_id,
        &budget,
        stopped,
        reason.as_deref(),
        usage_stored.id,
        failed.as_ref().map(|event| event.id),
    )
    .await?;
    tx.commit().await?;
    crate::usage_metrics::record(&entry);
    let mut emitted = vec![usage_stored];
    if let Some(failed) = failed {
        emitted.push(failed);
    }
    Ok((entry, emitted))
}

/// Stop a claimed run for going over budget, on the caller's transaction:
/// append `ClaimFailed` naming `holder` and dead-letter the run. The caller has
/// already taken the claim off `holder`, and `thread` is the row after that
/// write. Every budget stop comes through here, whether a usage report or the
/// claim reaper caught it, so a supervisor sees one kind of stop.
pub(super) async fn fail_claim_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread: Thread,
    holder: MemberId,
    reason: BudgetReason,
    budget: &ThreadBudget,
) -> Result<StoredEvent, StoreError> {
    let thread_id = thread.id;
    let reason = reason.as_str().to_owned();
    let failed = events::append_in_tx(
        tx,
        &Event::ClaimFailed {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id,
            member_id: holder,
            reason: reason.clone(),
            thread,
        },
    )
    .await?;
    dlq::record_in_tx(
        tx,
        &NewDlqEntry {
            workspace_id,
            channel_id,
            thread_id,
            member_id: holder,
            reason,
            used_tokens: budget.used_tokens,
            used_usd_micros: budget.used_usd_micros,
            used_turns: budget.used_turns,
        },
    )
    .await?;
    Ok(failed)
}

/// Charge `worked_secs` of wall time to a thread's budget on the caller's
/// transaction, creating the row (with no maxima) as a usage report does, and
/// return the totals. `None` charges nothing and returns the budget as it
/// stands, or `None` when the thread has no budget row.
pub(super) async fn charge_wall_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    worked_secs: Option<i64>,
) -> Result<Option<ThreadBudget>, StoreError> {
    let row = match worked_secs {
        Some(secs) => Some(
            sqlx::query(&format!(
                "INSERT INTO maidan_thread_budgets (thread_id, used_wall_secs)
                 VALUES ($1, $2)
                 ON CONFLICT (thread_id) DO UPDATE SET
                     used_wall_secs = maidan_thread_budgets.used_wall_secs + excluded.used_wall_secs,
                     updated_at = now()
                 RETURNING {COLS}"
            ))
            .bind(thread_id.0)
            .bind(secs)
            .fetch_one(&mut **tx)
            .await?,
        ),
        None => {
            sqlx::query(&format!(
                "SELECT {COLS} FROM maidan_thread_budgets WHERE thread_id = $1"
            ))
            .bind(thread_id.0)
            .fetch_optional(&mut **tx)
            .await?
        }
    };
    Ok(row.as_ref().map(row_to_budget))
}

fn row_to_budget(row: &sqlx::postgres::PgRow) -> ThreadBudget {
    ThreadBudget {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        max_tokens: row.get::<Option<i64>, _>("max_tokens"),
        max_usd_micros: row.get::<Option<i64>, _>("max_usd_micros"),
        max_turns: row.get::<Option<i64>, _>("max_turns"),
        max_wall_secs: row.get::<Option<i64>, _>("max_wall_secs"),
        used_tokens: row.get::<i64, _>("used_tokens"),
        used_usd_micros: row.get::<i64, _>("used_usd_micros"),
        used_turns: row.get::<i64, _>("used_turns"),
        used_wall_secs: row.get::<i64, _>("used_wall_secs"),
        used_input_tokens: row.get::<i64, _>("used_input_tokens"),
        used_output_tokens: row.get::<i64, _>("used_output_tokens"),
        used_cache_read_tokens: row.get::<i64, _>("used_cache_read_tokens"),
        used_cache_write_5m_tokens: row.get::<i64, _>("used_cache_write_5m_tokens"),
        used_cache_write_1h_tokens: row.get::<i64, _>("used_cache_write_1h_tokens"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

/// Apply only the dimensions a patch names.
///
/// Read-and-write in one transaction: a read-modify-write in the caller would
/// let two orchestrators adjusting different dimensions clobber each other, and
/// the point of a patch is that they should not have to coordinate.
pub async fn patch_budget(
    pool: &PgPool,
    thread_id: ThreadId,
    patch: BudgetPatch,
) -> Result<ThreadBudget, StoreError> {
    let mut tx = pool.begin().await?;
    let current = sqlx::query(
        "SELECT max_tokens, max_usd_micros, max_turns, max_wall_secs
         FROM maidan_thread_budgets WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?;
    // No row yet: the patch applies to an empty envelope, so a first patch sets
    // exactly the dimensions it names and leaves the rest uncapped.
    let base = match current.as_ref() {
        Some(row) => BudgetLimits {
            max_tokens: row.get("max_tokens"),
            max_usd_micros: row.get("max_usd_micros"),
            max_turns: row.get("max_turns"),
            max_wall_secs: row.get("max_wall_secs"),
        },
        None => BudgetLimits::default(),
    };
    let merged = patch.apply(base);
    let row = sqlx::query(
        "INSERT INTO maidan_thread_budgets
             (thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
         ON CONFLICT (thread_id) DO UPDATE SET
             max_tokens = excluded.max_tokens,
             max_usd_micros = excluded.max_usd_micros,
             max_turns = excluded.max_turns,
             max_wall_secs = excluded.max_wall_secs,
             updated_at = excluded.updated_at
         RETURNING thread_id, max_tokens, max_usd_micros, max_turns, max_wall_secs, \
             used_tokens, used_usd_micros, used_turns, used_wall_secs, created_at, updated_at, used_input_tokens, used_output_tokens, used_cache_read_tokens, used_cache_write_5m_tokens, used_cache_write_1h_tokens",
    )
    .bind(thread_id.0)
    .bind(merged.max_tokens)
    .bind(merged.max_usd_micros)
    .bind(merged.max_turns)
    .bind(merged.max_wall_secs)

    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row_to_budget(&row))
}
