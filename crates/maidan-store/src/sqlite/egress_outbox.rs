//! Durable projector egress outbox (Cluster 377.1). SQLite twin of the Postgres
//! module — SQLite serializes writers (one connection, Cluster 277), so a
//! select-then-update in a transaction claims atomically without `FOR UPDATE SKIP
//! LOCKED`. All timestamps are store-bound rfc3339, so a plain `<=` comparison is
//! consistent.

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::StoreError;
use maidan_types::{EgressOutbox, EgressOutboxId, NewEgressOutbox, ThreadId, WorkspaceId};

pub async fn enqueue(
    pool: &SqlitePool,
    new: NewEgressOutbox,
) -> Result<Option<EgressOutboxId>, StoreError> {
    let id = EgressOutboxId::new();
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_egress_outbox
           (id, workspace_id, thread_id, source_log_id, surface, selector, body,
            status, attempts, next_attempt_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', 0, ?, ?, ?)
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
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| EgressOutboxId(r.get("id"))))
}

pub async fn claim_next_due(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    lease_secs: i64,
) -> Result<Option<EgressOutbox>, StoreError> {
    let now_s = now.to_rfc3339();
    let mut tx = pool.begin().await?;
    let candidate = sqlx::query(
        "SELECT id FROM maidan_egress_outbox
         WHERE status = 'pending' AND next_attempt_at <= ?
         ORDER BY next_attempt_at ASC
         LIMIT 1",
    )
    .bind(&now_s)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(cand) = candidate else {
        return Ok(None);
    };
    let id: Uuid = cand.get("id");
    let lease = (now + chrono::Duration::seconds(lease_secs)).to_rfc3339();
    let row = sqlx::query(
        "UPDATE maidan_egress_outbox
         SET attempts = attempts + 1, next_attempt_at = ?, updated_at = ?
         WHERE id = ?
         RETURNING id, workspace_id, thread_id, surface, selector, body, attempts",
    )
    .bind(&lease)
    .bind(&now_s)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(row_to_egress(&row)))
}

pub async fn mark_delivered(pool: &SqlitePool, id: EgressOutboxId) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE maidan_egress_outbox SET status = 'delivered', updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_failed(
    pool: &SqlitePool,
    id: EgressOutboxId,
    error: &str,
    retry_at: Option<DateTime<Utc>>,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    match retry_at {
        Some(t) => {
            sqlx::query(
                "UPDATE maidan_egress_outbox
                 SET status = 'pending', next_attempt_at = ?, last_error = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(t.to_rfc3339())
            .bind(error)
            .bind(&now)
            .bind(id.0)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE maidan_egress_outbox
                 SET status = 'dead', last_error = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(error)
            .bind(&now)
            .bind(id.0)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub async fn count_dead(pool: &SqlitePool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM maidan_egress_outbox WHERE status = 'dead'")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("c"))
}

fn row_to_egress(row: &sqlx::sqlite::SqliteRow) -> EgressOutbox {
    EgressOutbox {
        id: EgressOutboxId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        thread_id: ThreadId(row.get("thread_id")),
        surface: row.get("surface"),
        selector: row.get("selector"),
        body: row.get("body"),
        attempts: row.get("attempts"),
    }
}
