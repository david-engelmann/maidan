//! The usage-meter daily rollup (`maidan_types::usage_meter`). A flush adds
//! to each day's quantity, so two replicas flushing the same day both count.

use chrono::NaiveDate;
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{UsageMeter, UsageMeterDay, WorkspaceId};

/// A row whose workspace is gone (erased between the count and the flush)
/// is dropped, not written: one dead workspace must not fail the batch for
/// every live one.
pub async fn add(pool: &SqlitePool, batch: &[UsageMeterDay]) -> Result<(), StoreError> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for row in batch {
        let written = sqlx::query(
            "INSERT INTO maidan_usage_meters (workspace_id, day, meter, quantity)
             SELECT id, ?2, ?3, ?4 FROM maidan_workspaces WHERE id = ?1
             ON CONFLICT (workspace_id, day, meter)
             DO UPDATE SET quantity = maidan_usage_meters.quantity + EXCLUDED.quantity",
        )
        .bind(row.workspace_id.0)
        .bind(row.day.to_string())
        .bind(row.meter.as_str())
        .bind(row.quantity)
        .execute(&mut *tx)
        .await?;
        if written.rows_affected() == 0 {
            tracing::debug!(workspace_id = %row.workspace_id.0, meter = row.meter.as_str(), "usage meters: workspace gone; count dropped");
        }
    }
    tx.commit().await?;
    Ok(())
}

/// `workspace_id`'s meters from `since` on, oldest day first.
pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    since: NaiveDate,
) -> Result<Vec<UsageMeterDay>, StoreError> {
    let rows = sqlx::query(
        "SELECT workspace_id, day, meter, quantity FROM maidan_usage_meters
         WHERE workspace_id = ?1 AND day >= ?2 ORDER BY day, meter",
    )
    .bind(workspace_id.0)
    .bind(since.to_string())
    .fetch_all(pool)
    .await?;
    rows.iter()
        .filter_map(|row| row_to_day(row).transpose())
        .collect()
}

/// A row, or `None` for a meter this build does not know (one a newer
/// build wrote), which is skipped rather than failing the read.
fn row_to_day(row: &sqlx::sqlite::SqliteRow) -> Result<Option<UsageMeterDay>, StoreError> {
    let meter: String = row.try_get("meter")?;
    let Some(meter) = UsageMeter::parse(&meter) else {
        return Ok(None);
    };
    Ok(Some(UsageMeterDay {
        workspace_id: WorkspaceId(row.try_get("workspace_id")?),
        day: row
            .try_get::<String, _>("day")?
            .parse()
            .map_err(|e: chrono::ParseError| StoreError::from(sqlx::Error::Decode(Box::new(e))))?,
        meter,
        quantity: row.try_get("quantity")?,
    }))
}

/// Replace each row's day value: for a gauge such as stored bytes, whose day
/// is its latest measurement.
pub async fn set(pool: &SqlitePool, batch: &[UsageMeterDay]) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    for row in batch {
        sqlx::query(
            "INSERT INTO maidan_usage_meters (workspace_id, day, meter, quantity)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (workspace_id, day, meter) DO UPDATE SET quantity = EXCLUDED.quantity",
        )
        .bind(row.workspace_id.0)
        .bind(row.day.to_string())
        .bind(row.meter.as_str())
        .bind(row.quantity)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Live workspaces with no `meter` row for `day`, at most `limit`: the ones a
/// daily measurement has still to reach.
pub async fn unmeasured(
    pool: &SqlitePool,
    day: NaiveDate,
    meter: UsageMeter,
    limit: i64,
) -> Result<Vec<WorkspaceId>, StoreError> {
    let rows = sqlx::query(
        "SELECT w.id FROM maidan_workspaces w
         WHERE w.tombstoned_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM maidan_usage_meters u
                           WHERE u.workspace_id = w.id AND u.day = ?1 AND u.meter = ?2)
         ORDER BY w.id LIMIT ?3",
    )
    .bind(day.to_string())
    .bind(meter.as_str())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| row.try_get("id").map(WorkspaceId).map_err(Into::into))
        .collect()
}

/// What `workspace_id` stores now: the bytes of every artifact it refers to,
/// each counted once, and of its live messages' bodies.
pub async fn stored_bytes(pool: &SqlitePool, workspace_id: WorkspaceId) -> Result<i64, StoreError> {
    let artifacts: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(a.size_bytes), 0)
         FROM maidan_artifact_refs r JOIN maidan_artifacts a ON a.sha256 = r.sha256
         WHERE r.workspace_id = ?1",
    )
    .bind(workspace_id.0)
    .fetch_one(pool)
    .await?;
    let messages: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(length(CAST(m.body AS BLOB))), 0)
         FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?1 AND m.tombstoned_at IS NULL",
    )
    .bind(workspace_id.0)
    .fetch_one(pool)
    .await?;
    Ok(artifacts.saturating_add(messages))
}
