//! A workspace's own retention (`maidan_retention_policies`). The sweeper's
//! per-workspace prunes are in `retention.rs`.

use maidan_types::{RetentionDays, WorkspaceId};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_days(row: &sqlx::postgres::PgRow) -> RetentionDays {
    RetentionDays {
        messages_days: row.get("messages_days"),
        events_days: row.get("events_days"),
        deliveries_days: row.get("deliveries_days"),
    }
}

/// What the workspace set; all `None` when it has set nothing.
pub async fn get(pool: &PgPool, workspace_id: WorkspaceId) -> Result<RetentionDays, StoreError> {
    let mut conn = pool.acquire().await?;
    get_on(&mut conn, workspace_id).await
}

async fn get_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<RetentionDays, StoreError> {
    let row = sqlx::query(
        "SELECT messages_days, events_days, deliveries_days
         FROM maidan_retention_policies WHERE workspace_id = $1",
    )
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.as_ref().map(row_to_days).unwrap_or_default())
}

/// Replace the workspace's retention, checked against what the instance keeps,
/// with its audit row in the same transaction (D-A). Nothing set removes the
/// row.
pub async fn set_audited(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    days: RetentionDays,
    instance: RetentionDays,
    audit: crate::AuditFor<RetentionDays>,
) -> Result<RetentionDays, StoreError> {
    crate::retention_policy::validate(&days, &instance)?;
    let mut tx = pool.begin().await?;
    if days.is_unset() {
        sqlx::query("DELETE FROM maidan_retention_policies WHERE workspace_id = $1")
            .bind(workspace_id.0)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO maidan_retention_policies
                 (workspace_id, messages_days, events_days, deliveries_days, updated_at)
             VALUES ($1, $2, $3, $4, NOW())
             ON CONFLICT (workspace_id) DO UPDATE
             SET messages_days = excluded.messages_days,
                 events_days = excluded.events_days,
                 deliveries_days = excluded.deliveries_days,
                 updated_at = excluded.updated_at",
        )
        .bind(workspace_id.0)
        .bind(days.messages_days)
        .bind(days.events_days)
        .bind(days.deliveries_days)
        .execute(&mut *tx)
        .await?;
    }
    let stored = get_on(&mut tx, workspace_id).await?;
    super::audit::append_counted(&mut tx, audit(&stored)).await?;
    tx.commit().await?;
    Ok(stored)
}

/// Every workspace that has set a retention.
pub async fn list(pool: &PgPool) -> Result<Vec<(WorkspaceId, RetentionDays)>, StoreError> {
    let rows = sqlx::query(
        "SELECT workspace_id, messages_days, events_days, deliveries_days
         FROM maidan_retention_policies ORDER BY workspace_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                WorkspaceId(row.get::<Uuid, _>("workspace_id")),
                row_to_days(row),
            )
        })
        .collect())
}
