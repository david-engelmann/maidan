//! Allowance crossings (`maidan_types::usage_meter`): marked once per
//! workspace, meter and month, with the `UsageOverAllowance` event and a
//! notification to each admin in the same transaction.

use chrono::Utc;
use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{
    Event, EventKind, MemberId, NotificationId, OverAllowance, StoredEvent, UsageMeter, WorkspaceId,
};

/// Mark `mark` unless that workspace, meter and month is marked already, and
/// then append its event and notify `admins`. `None` when it was marked
/// already (another replica, or an earlier tick), and nothing is written.
pub async fn mark(
    pool: &PgPool,
    mark: &OverAllowance,
    admins: &[MemberId],
) -> Result<Option<StoredEvent>, StoreError> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO maidan_usage_over_allowance
            (workspace_id, meter, month, quantity, allowance, marked_at)
         SELECT id, $2, $3, $4, $5, $6 FROM maidan_workspaces WHERE id = $1
         ON CONFLICT (workspace_id, meter, month) DO NOTHING",
    )
    .bind(mark.workspace_id.0)
    .bind(mark.meter.as_str())
    .bind(&mark.month)
    .bind(mark.quantity)
    .bind(mark.allowance)
    .bind(mark.marked_at)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(None);
    }
    let event = Event::UsageOverAllowance {
        occurred_at: mark.marked_at,
        workspace_id: mark.workspace_id,
        meter: mark.meter,
        month: mark.month.clone(),
        quantity: mark.quantity,
        allowance: mark.allowance,
    };
    let stored = super::events::append_in_tx(&mut tx, &event).await?;
    for admin in admins {
        sqlx::query(
            "INSERT INTO maidan_notifications
                (id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                 message_id, actor_id, created_at, read_at, snoozed_until)
             VALUES ($1, $2, $3, $4, $5, NULL, NULL, NULL, NULL, $6, NULL, NULL)
             ON CONFLICT (member_id, source_log_id) DO NOTHING",
        )
        .bind(NotificationId::new().0)
        .bind(mark.workspace_id.0)
        .bind(admin.0)
        .bind(EventKind::UsageOverAllowance.as_str())
        .bind(stored.id)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Some(stored))
}

/// The crossings in `month`, for one workspace or (operator) every one.
pub async fn list(
    pool: &PgPool,
    workspace_id: Option<WorkspaceId>,
    month: &str,
) -> Result<Vec<OverAllowance>, StoreError> {
    let rows = sqlx::query(
        "SELECT workspace_id, meter, month, quantity, allowance, marked_at
         FROM maidan_usage_over_allowance
         WHERE month = $1 AND ($2::UUID IS NULL OR workspace_id = $2)
         ORDER BY workspace_id, meter",
    )
    .bind(month)
    .bind(workspace_id.map(|w| w.0))
    .fetch_all(pool)
    .await?;
    rows.iter()
        .filter_map(|row| row_to_mark(row).transpose())
        .collect()
}

/// A row, or `None` for a meter this build does not know.
fn row_to_mark(row: &sqlx::postgres::PgRow) -> Result<Option<OverAllowance>, StoreError> {
    let meter: String = row.try_get("meter")?;
    let Some(meter) = UsageMeter::parse(&meter) else {
        return Ok(None);
    };
    Ok(Some(OverAllowance {
        workspace_id: WorkspaceId(row.try_get("workspace_id")?),
        meter,
        month: row.try_get("month")?,
        quantity: row.try_get("quantity")?,
        allowance: row.try_get("allowance")?,
        marked_at: row.try_get("marked_at")?,
    }))
}
