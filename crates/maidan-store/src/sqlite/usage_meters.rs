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
