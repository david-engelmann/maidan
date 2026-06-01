use chrono::{DateTime, Utc};
use maidan_types::{
    AutomationDelivery, AutomationDeliveryPending, AutomationSourceKind, NewAutomationDelivery,
    WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::automation_deliveries::AutomationDeliveryFilter;
use crate::error::StoreError;

const PENDING: &str = "delivered_at IS NULL AND quarantined_at IS NULL";

pub async fn enqueue(pool: &SqlitePool, new: NewAutomationDelivery) -> Result<i64, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_automation_deliveries
            (workspace_id, source_kind, source_id, target_url, header_name, header_value, payload, next_attempt_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(new.workspace_id.0)
    .bind(new.source_kind.as_str())
    .bind(new.source_id)
    .bind(&new.target_url)
    .bind(&new.header_name)
    .bind(&new.header_value)
    .bind(&new.payload)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn list_pending(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<AutomationDeliveryPending>, StoreError> {
    let now = Utc::now().to_rfc3339();
    let rows = sqlx::query(
        "SELECT id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                payload, attempts
         FROM maidan_automation_deliveries
         WHERE delivered_at IS NULL
           AND quarantined_at IS NULL
           AND next_attempt_at <= ?
         ORDER BY id ASC
         LIMIT ?",
    )
    .bind(&now)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_pending).collect()
}

pub async fn list_for_workspace(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    filter: AutomationDeliveryFilter,
    limit: i64,
) -> Result<Vec<AutomationDelivery>, StoreError> {
    let predicate = match filter {
        AutomationDeliveryFilter::Pending => PENDING,
        AutomationDeliveryFilter::DeadLetter => "quarantined_at IS NOT NULL",
        AutomationDeliveryFilter::Delivered => "delivered_at IS NOT NULL",
    };
    let rows = sqlx::query(&format!(
        "SELECT id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                attempts, last_error, delivered_at, quarantined_at, next_attempt_at, created_at
         FROM maidan_automation_deliveries
         WHERE workspace_id = ? AND {predicate}
         ORDER BY id DESC
         LIMIT ?"
    ))
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_delivery).collect()
}

pub async fn get(
    pool: &SqlitePool,
    delivery_id: i64,
    workspace_id: WorkspaceId,
) -> Result<AutomationDelivery, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                attempts, last_error, delivered_at, quarantined_at, next_attempt_at, created_at
         FROM maidan_automation_deliveries
         WHERE id = ? AND workspace_id = ?",
    )
    .bind(delivery_id)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_delivery(&row)
}

pub async fn mark_delivered(pool: &SqlitePool, delivery_id: i64) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET delivered_at = datetime('now')
         WHERE id = ? AND delivered_at IS NULL",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn record_attempt(
    pool: &SqlitePool,
    delivery_id: i64,
    error: &str,
    next_attempt_at: DateTime<Utc>,
) -> Result<i32, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET attempts = attempts + 1,
             last_error = ?,
             next_attempt_at = ?
         WHERE id = ?
         RETURNING attempts",
    )
    .bind(error)
    .bind(next_attempt_at.to_rfc3339())
    .bind(delivery_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    Ok(row.get("attempts"))
}

pub async fn quarantine(pool: &SqlitePool, delivery_id: i64) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET quarantined_at = datetime('now')
         WHERE id = ? AND quarantined_at IS NULL",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn replay(
    pool: &SqlitePool,
    delivery_id: i64,
    workspace_id: WorkspaceId,
) -> Result<AutomationDelivery, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET quarantined_at = NULL,
             delivered_at = NULL,
             next_attempt_at = ?,
             attempts = 0,
             last_error = NULL
         WHERE id = ? AND workspace_id = ? AND quarantined_at IS NOT NULL
         RETURNING id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                   attempts, last_error, delivered_at, quarantined_at, next_attempt_at, created_at",
    )
    .bind(&now)
    .bind(delivery_id)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_delivery(&row)
}

fn parse_source_kind(s: &str) -> Result<AutomationSourceKind, StoreError> {
    AutomationSourceKind::parse(s)
        .ok_or_else(|| StoreError::InvalidInput(format!("bad automation source_kind: {s}")))
}

fn parse_uuid_from_row(row: &sqlx::sqlite::SqliteRow, col: &str) -> Result<Uuid, StoreError> {
    Ok(row.get::<Uuid, _>(col))
}

fn row_to_pending(row: &sqlx::sqlite::SqliteRow) -> Result<AutomationDeliveryPending, StoreError> {
    let source_kind: String = row.get("source_kind");
    Ok(AutomationDeliveryPending {
        id: row.get("id"),
        workspace_id: WorkspaceId(parse_uuid_from_row(row, "workspace_id")?),
        source_kind: parse_source_kind(&source_kind)?,
        source_id: parse_uuid_from_row(row, "source_id")?,
        target_url: row.get("target_url"),
        header_name: row.get("header_name"),
        header_value: row.get("header_value"),
        payload: row.get("payload"),
        attempts: row.get("attempts"),
    })
}

fn row_to_delivery(row: &sqlx::sqlite::SqliteRow) -> Result<AutomationDelivery, StoreError> {
    let source_kind: String = row.get("source_kind");
    Ok(AutomationDelivery {
        id: row.get("id"),
        workspace_id: WorkspaceId(parse_uuid_from_row(row, "workspace_id")?),
        source_kind: parse_source_kind(&source_kind)?,
        source_id: parse_uuid_from_row(row, "source_id")?,
        target_url: row.get("target_url"),
        header_name: row.get("header_name"),
        header_value: row.get("header_value"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        delivered_at: row.get("delivered_at"),
        quarantined_at: row.get("quarantined_at"),
        next_attempt_at: row.get::<DateTime<Utc>, _>("next_attempt_at"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}
