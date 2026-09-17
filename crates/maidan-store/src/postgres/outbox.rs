//! Postgres transactional outbox rows relayed to `PostgresBus` after commit.

use maidan_types::WorkspaceId;
use sqlx::{PgPool, Row};

use crate::error::StoreError;

const RELAYABLE: &str = "published_at IS NULL AND quarantined_at IS NULL";

#[derive(Debug, Clone)]
pub struct OutboxRow {
    pub id: i64,
    pub log_id: i64,
    pub attempts: i32,
    /// The event payload, JOINed from `maidan_events` so the relay publishes
    /// directly from the pending list instead of a per-row `get_stored_event`
    /// round-trip.
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuarantinedOutboxRow {
    pub id: i64,
    pub log_id: i64,
    pub attempts: i32,
    pub quarantined_at: chrono::DateTime<chrono::Utc>,
}

/// Atomically claim up to `limit` relayable rows for this relay.
///
/// The relay is spawned in **every** replica and `validate_startup` refuses to
/// disable it in production, so an unlocked `SELECT` meant every replica
/// relayed every row: N POSTs to each tenant webhook (there is no unique on
/// `(subscription_id, log_id)`) and N `fsm_hook` firings through
/// `dispatch_mcp_tool` with `AuthContext::bypass()`.
///
/// `FOR UPDATE SKIP LOCKED` inside a CTE is the same shape `claim_next_thread`
/// and `claim_next_due_schedule` already use: concurrent claimers take disjoint
/// rows rather than blocking. The lease is what makes a crash safe — a
/// claimed-but-unpublished row becomes claimable again after `lease_secs`, so
/// **at-least-once is preserved: a claim is not a publish.**
pub async fn claim_pending(
    pool: &PgPool,
    limit: i64,
    lease_secs: i64,
) -> Result<Vec<OutboxRow>, StoreError> {
    let rows = sqlx::query(&format!(
        "WITH candidate AS (
             SELECT o.id
             FROM maidan_outbox o
             WHERE {RELAYABLE}
               AND (o.claimed_at IS NULL
                    OR o.claimed_at < NOW() - make_interval(secs => $2::double precision))
             ORDER BY o.id ASC
             LIMIT $1
             FOR UPDATE SKIP LOCKED
         ), claimed AS (
             UPDATE maidan_outbox o
             SET claimed_at = NOW()
             FROM candidate c
             WHERE o.id = c.id
             RETURNING o.id, o.log_id, o.attempts
         )
         SELECT c.id, c.log_id, c.attempts, e.payload
         FROM claimed c
         JOIN maidan_events e ON e.id = c.log_id
         ORDER BY c.id ASC"
    ))
    .bind(limit)
    .bind(lease_secs as f64)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| OutboxRow {
            id: row.get("id"),
            log_id: row.get("log_id"),
            attempts: row.get("attempts"),
            payload: row.get("payload"),
        })
        .collect())
}

pub async fn list_pending(pool: &PgPool, limit: i64) -> Result<Vec<OutboxRow>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT o.id, o.log_id, o.attempts, e.payload
         FROM maidan_outbox o
         JOIN maidan_events e ON e.id = o.log_id
         WHERE {RELAYABLE}
         ORDER BY o.id ASC
         LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| OutboxRow {
            id: row.get("id"),
            log_id: row.get("log_id"),
            attempts: row.get("attempts"),
            payload: row.get("payload"),
        })
        .collect())
}

pub async fn mark_published(pool: &PgPool, outbox_id: i64) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_outbox
         SET published_at = NOW()
         WHERE id = $1 AND published_at IS NULL",
    )
    .bind(outbox_id)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Mark a batch of outbox rows published in a single statement. Idempotent:
/// rows already published are skipped by the `IS NULL` guard, so no error on a
/// partial match. A no-op for an empty slice.
pub async fn mark_published_batch(pool: &PgPool, outbox_ids: &[i64]) -> Result<(), StoreError> {
    if outbox_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE maidan_outbox
         SET published_at = NOW()
         WHERE id = ANY($1) AND published_at IS NULL",
    )
    .bind(outbox_ids)
    .execute(pool)
    .await?;
    Ok(())
}

/// Increments `attempts` and returns the new value. Count a failed relay
/// attempt and **release the claim**, so the row is retryable on the next tick
/// rather than waiting out its lease. The lease exists to survive a crashed
/// claimer, not to pace retries — `attempts` and the quarantine ceiling already
/// do that.
pub async fn record_attempt(pool: &PgPool, outbox_id: i64) -> Result<i32, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_outbox
         SET attempts = attempts + 1, claimed_at = NULL
         WHERE id = $1
         RETURNING attempts",
    )
    .bind(outbox_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    Ok(row.get("attempts"))
}

pub async fn quarantine(pool: &PgPool, outbox_id: i64) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_outbox
         SET quarantined_at = NOW()
         WHERE id = $1 AND quarantined_at IS NULL",
    )
    .bind(outbox_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Clears quarantine so the relay can retry; row must belong to `workspace_id`.
pub async fn replay_quarantined(
    pool: &PgPool,
    outbox_id: i64,
    workspace_id: WorkspaceId,
) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_outbox o
         SET quarantined_at = NULL, attempts = 0
         FROM maidan_events e
         WHERE o.log_id = e.id
           AND o.id = $1
           AND e.workspace_id = $2
           AND o.published_at IS NULL
           AND o.quarantined_at IS NOT NULL",
    )
    .bind(outbox_id)
    .bind(workspace_id.0)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn count_pending(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT COUNT(*)::bigint AS n FROM maidan_outbox WHERE {RELAYABLE}"
    ))
    .fetch_one(pool)
    .await?;
    Ok(row.get("n"))
}

pub async fn list_quarantined_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<QuarantinedOutboxRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT o.id, o.log_id, o.attempts, o.quarantined_at
         FROM maidan_outbox o
         INNER JOIN maidan_events e ON e.id = o.log_id
         WHERE e.workspace_id = $1
           AND o.published_at IS NULL
           AND o.quarantined_at IS NOT NULL
         ORDER BY o.id DESC
         LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| QuarantinedOutboxRow {
            id: row.get("id"),
            log_id: row.get("log_id"),
            attempts: row.get("attempts"),
            quarantined_at: row.get("quarantined_at"),
        })
        .collect())
}

pub async fn count_quarantined(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*)::bigint AS n
         FROM maidan_outbox
         WHERE published_at IS NULL AND quarantined_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok(row.get("n"))
}

/// Seconds since the oldest relayable pending row was created; `None` when none pending.
pub async fn oldest_relayable_pending_age_secs(pool: &PgPool) -> Result<Option<f64>, StoreError> {
    let age: Option<f64> = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM (NOW() - MIN(created_at)))
         FROM maidan_outbox
         WHERE published_at IS NULL AND quarantined_at IS NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok(age)
}

pub async fn enqueue_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    log_id: i64,
) -> Result<(), StoreError> {
    sqlx::query("INSERT INTO maidan_outbox (log_id) VALUES ($1)")
        .bind(log_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
