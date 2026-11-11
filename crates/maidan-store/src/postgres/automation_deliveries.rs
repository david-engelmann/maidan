use chrono::{DateTime, Utc};
use maidan_types::{
    AutomationDelivery, AutomationDeliveryPending, AutomationSourceKind, NewAutomationDelivery,
    WorkspaceId,
};
use sqlx::{PgPool, Row};

use crate::automation_deliveries::AutomationDeliveryFilter;
use crate::error::StoreError;

const PENDING: &str = "delivered_at IS NULL AND quarantined_at IS NULL";

/// Enqueue a delivery. One made for an event (`log_id`) is enqueued once per
/// source and event, however many replicas see the event: `None` when it was
/// already there. Its `created_at` is the event's `occurred_at`, so the unique
/// `(source_kind, source_id, log_id, created_at)` holds across partitions
/// (0234); a delivery with no event is never a duplicate.
pub async fn enqueue(pool: &PgPool, new: NewAutomationDelivery) -> Result<Option<i64>, StoreError> {
    let (traceparent, tracestate) = crate::trace::current_columns();
    let row = sqlx::query(
        "INSERT INTO maidan_automation_deliveries
            (workspace_id, source_kind, source_id, target_url, header_name, header_value, payload,
             traceparent, tracestate, log_id, created_at)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, COALESCE(e.occurred_at, NOW())
         FROM (SELECT 1) AS one
         LEFT JOIN maidan_events e ON e.id = $10
         ON CONFLICT (source_kind, source_id, log_id, created_at) DO NOTHING
         RETURNING id",
    )
    .bind(new.workspace_id.0)
    .bind(new.source_kind.as_str())
    .bind(new.source_id)
    .bind(&new.target_url)
    .bind(&new.header_name)
    .bind(&new.header_value)
    .bind(&new.payload)
    .bind(&traceparent)
    .bind(&tracestate)
    .bind(new.log_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| row.get("id")))
}

pub async fn list_pending(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<AutomationDeliveryPending>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT id, workspace_id, source_kind, source_id, target_url, header_name, header_value,
                payload, attempts, traceparent, tracestate
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

/// Claim the oldest due delivery: lease it `lease_secs` forward so no other
/// replica's poller takes it meanwhile. The send's outcome replaces the lease;
/// a poller that dies mid-send leaves the row due again when it runs out.
pub async fn claim_next(
    pool: &PgPool,
    lease_secs: i64,
) -> Result<Option<AutomationDeliveryPending>, StoreError> {
    let row = sqlx::query(&format!(
        "WITH due AS (
             SELECT id, created_at FROM maidan_automation_deliveries
             WHERE {PENDING} AND next_attempt_at <= NOW()
             ORDER BY id ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_automation_deliveries d
         SET next_attempt_at = NOW() + make_interval(secs => $1)
         FROM due
         WHERE d.id = due.id AND d.created_at = due.created_at
         RETURNING d.id, d.workspace_id, d.source_kind, d.source_id, d.target_url, d.header_name,
                   d.header_value, d.payload, d.attempts, d.traceparent, d.tracestate"
    ))
    .bind(lease_secs as f64)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_pending).transpose()
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

/// Move a pending delivery's next attempt without counting one.
pub async fn defer(
    pool: &PgPool,
    delivery_id: i64,
    next_attempt_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(&format!(
        "UPDATE maidan_automation_deliveries
         SET next_attempt_at = $2
         WHERE id = $1 AND {PENDING}"
    ))
    .bind(delivery_id)
    .bind(next_attempt_at)
    .execute(pool)
    .await?;
    Ok(())
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
        trace: maidan_types::TraceContext::from_columns(
            row.try_get("traceparent").unwrap_or(None),
            row.try_get("tracestate").unwrap_or(None),
        ),
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
