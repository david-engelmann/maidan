//! Durable projector egress outbox (Cluster 377.1): enqueue a message bound for an
//! external surface and let a retry/backoff worker claim + deliver it, instead of
//! the projectors' best-effort inline post. See the SQLite twin.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{
    DeadEgress, EgressKind, EgressOutbox, EgressOutboxId, NewEgressOutbox, ThreadId, WorkspaceId,
};

/// Enqueue a delivery: `pending`, due now. Returns `None` when an identical
/// `(source_log_id, surface, selector)` row already exists — every replica runs
/// the router that enqueues, so the dedup index is what keeps a 3-replica deploy
/// from posting the same comment three times.
pub async fn enqueue(
    pool: &PgPool,
    new: NewEgressOutbox,
) -> Result<Option<EgressOutboxId>, StoreError> {
    let id = EgressOutboxId::new();
    let row = sqlx::query(
        "INSERT INTO maidan_egress_outbox
           (id, workspace_id, thread_id, source_log_id, surface, selector, body, kind,
            status, attempts, next_attempt_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', 0, now(), now(), now())
         ON CONFLICT (source_log_id, surface, selector) DO NOTHING
         RETURNING id",
    )
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.thread_id.0)
    .bind(new.source_log_id)
    .bind(new.target.surface().as_str())
    .bind(new.target.selector())
    .bind(&new.body)
    .bind(new.kind.as_str())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| EgressOutboxId(r.get("id"))))
}

/// Atomically claim the oldest due pending row: lease it forward
/// (`next_attempt_at = now + lease_secs`) and bump `attempts`, so a worker that
/// crashes mid-send releases the row after the lease. `FOR UPDATE SKIP LOCKED`
/// lets concurrent replicas claim distinct rows.
pub async fn claim_next_due(
    pool: &PgPool,
    now: DateTime<Utc>,
    lease_secs: i64,
) -> Result<Option<EgressOutbox>, StoreError> {
    let row = sqlx::query(
        "WITH due AS (
             SELECT id FROM maidan_egress_outbox
             WHERE status = 'pending' AND next_attempt_at <= $1
             ORDER BY next_attempt_at ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_egress_outbox e
         SET attempts = e.attempts + 1,
             next_attempt_at = $1 + make_interval(secs => $2),
             updated_at = now()
         FROM due
         WHERE e.id = due.id
         RETURNING e.id, e.workspace_id, e.thread_id, e.surface, e.selector, e.body, e.attempts, e.kind",
    )
    .bind(now)
    .bind(lease_secs as f64)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_egress))
}

/// Mark a claimed delivery delivered.
pub async fn mark_delivered(pool: &PgPool, id: EgressOutboxId) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_egress_outbox SET status = 'delivered', updated_at = now() WHERE id = $1",
    )
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

/// On a failed send: `retry_at = Some(t)` reschedules (stays `pending`,
/// `next_attempt_at = t`); `None` dead-letters (`status = 'dead'`). The worker
/// decides, from the attempt count and the error class.
pub async fn mark_failed(
    pool: &PgPool,
    id: EgressOutboxId,
    error: &str,
    retry_at: Option<DateTime<Utc>>,
) -> Result<(), StoreError> {
    match retry_at {
        Some(t) => {
            sqlx::query(
                "UPDATE maidan_egress_outbox
                 SET status = 'pending', next_attempt_at = $2, last_error = $3, updated_at = now()
                 WHERE id = $1",
            )
            .bind(id.0)
            .bind(t)
            .bind(error)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE maidan_egress_outbox
                 SET status = 'dead', last_error = $2, updated_at = now()
                 WHERE id = $1",
            )
            .bind(id.0)
            .bind(error)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Count dead-lettered deliveries (DLQ depth) — for metrics / ops.
pub async fn count_dead(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM maidan_egress_outbox WHERE status = 'dead'")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("c"))
}

/// List dead-lettered deliveries, newest-updated first (the operator DLQ view).
pub async fn list_dead(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<DeadEgress>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, thread_id, surface, selector, attempts, last_error, updated_at
         FROM maidan_egress_outbox
         WHERE status = 'dead' AND workspace_id = $1
         ORDER BY updated_at DESC
         LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_dead).collect())
}

/// Requeue a dead delivery for a fresh attempt: `pending`, due now, `attempts`
/// reset. Returns whether a dead row was actually requeued.
pub async fn requeue_dead(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    id: EgressOutboxId,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_egress_outbox
         SET status = 'pending', attempts = 0, next_attempt_at = now(), updated_at = now()
         WHERE id = $1 AND status = 'dead' AND workspace_id = $2",
    )
    .bind(id.0)
    .bind(workspace_id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_dead(row: &sqlx::postgres::PgRow) -> DeadEgress {
    DeadEgress {
        id: EgressOutboxId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        thread_id: ThreadId(row.get("thread_id")),
        surface: row.get("surface"),
        selector: row.get("selector"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_egress(row: &sqlx::postgres::PgRow) -> EgressOutbox {
    EgressOutbox {
        id: EgressOutboxId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        thread_id: ThreadId(row.get("thread_id")),
        surface: row.get("surface"),
        selector: row.get("selector"),
        body: row.get("body"),
        attempts: row.get("attempts"),
        kind: EgressKind::parse(&row.get::<String, _>("kind")),
    }
}
