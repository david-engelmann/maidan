use chrono::{DateTime, Utc};
use maidan_types::{
    AutomationDelivery, AutomationDeliveryPending, AutomationSourceKind, NewAutomationDelivery,
    WorkspaceId,
};
use sqlx::{PgPool, Row};

use crate::automation_deliveries::AutomationDeliveryFilter;
use crate::error::StoreError;

const PENDING: &str = "delivered_at IS NULL AND quarantined_at IS NULL";

pub async fn enqueue(pool: &PgPool, new: NewAutomationDelivery) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_automation_deliveries
            (workspace_id, source_kind, source_id, target_url, header_name, header_value, payload)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(new.workspace_id.0)
    .bind(new.source_kind.as_str())
    .bind(new.source_id)
    .bind(&new.target_url)
    .bind(&new.header_name)
    .bind(&new.header_value)
    .bind(&new.payload)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn list_pending(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<AutomationDeliveryPending>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                payload, attempts
         FROM maidan_automation_deliveries
         WHERE {PENDING} AND next_attempt_at <= NOW()
         ORDER BY id ASC
         LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_pending).collect()
}

pub async fn list_for_workspace(
    pool: &PgPool,
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
         WHERE workspace_id = $1 AND {predicate}
         ORDER BY id DESC
         LIMIT $2"
    ))
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_delivery).collect()
}

pub async fn get(
    pool: &PgPool,
    delivery_id: i64,
    workspace_id: WorkspaceId,
) -> Result<AutomationDelivery, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                attempts, last_error, delivered_at, quarantined_at, next_attempt_at, created_at
         FROM maidan_automation_deliveries
         WHERE id = $1 AND workspace_id = $2",
    )
    .bind(delivery_id)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_delivery(&row)
}

pub async fn mark_delivered(pool: &PgPool, delivery_id: i64) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET delivered_at = NOW()
         WHERE id = $1 AND delivered_at IS NULL",
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
    pool: &PgPool,
    delivery_id: i64,
    error: &str,
    next_attempt_at: DateTime<Utc>,
) -> Result<i32, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET attempts = attempts + 1,
             last_error = $2,
             next_attempt_at = $3
         WHERE id = $1
         RETURNING attempts",
    )
    .bind(delivery_id)
    .bind(error)
    .bind(next_attempt_at)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    Ok(row.get("attempts"))
}

pub async fn quarantine(pool: &PgPool, delivery_id: i64) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET quarantined_at = NOW()
         WHERE id = $1 AND quarantined_at IS NULL",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn replay(
    pool: &PgPool,
    delivery_id: i64,
    workspace_id: WorkspaceId,
) -> Result<AutomationDelivery, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_automation_deliveries
         SET quarantined_at = NULL,
             delivered_at = NULL,
             next_attempt_at = NOW(),
             attempts = 0,
             last_error = NULL
         WHERE id = $1 AND workspace_id = $2 AND quarantined_at IS NOT NULL
         RETURNING id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                   attempts, last_error, delivered_at, quarantined_at, next_attempt_at, created_at",
    )
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

fn row_to_pending(row: &sqlx::postgres::PgRow) -> Result<AutomationDeliveryPending, StoreError> {
    let source_kind: String = row.get("source_kind");
    Ok(AutomationDeliveryPending {
        id: row.get("id"),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        source_kind: parse_source_kind(&source_kind)?,
        source_id: row.get("source_id"),
        target_url: row.get("target_url"),
        header_name: row.get("header_name"),
        header_value: row.get("header_value"),
        payload: row.get("payload"),
        attempts: row.get("attempts"),
    })
}

fn row_to_delivery(row: &sqlx::postgres::PgRow) -> Result<AutomationDelivery, StoreError> {
    let source_kind: String = row.get("source_kind");
    Ok(AutomationDelivery {
        id: row.get("id"),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        source_kind: parse_source_kind(&source_kind)?,
        source_id: row.get("source_id"),
        target_url: row.get("target_url"),
        header_name: row.get("header_name"),
        header_value: row.get("header_value"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        delivered_at: row.get("delivered_at"),
        quarantined_at: row.get("quarantined_at"),
        next_attempt_at: row.get("next_attempt_at"),
        created_at: row.get("created_at"),
    })
}
