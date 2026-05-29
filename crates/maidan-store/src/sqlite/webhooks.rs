use chrono::{DateTime, Utc};
use maidan_types::{
    NewWebhookSubscription, WebhookSubscription, WebhookSubscriptionDelivery,
    WebhookSubscriptionId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const SUB_COLS: &str =
    "id, workspace_id, url, label, event_kinds, secret_ciphertext, enabled, created_at, revoked_at";

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionRow {
    pub subscription: WebhookSubscription,
    pub secret_ciphertext: String,
}

pub async fn create(
    pool: &SqlitePool,
    new: NewWebhookSubscription,
) -> Result<WebhookSubscription, StoreError> {
    let id = Uuid::new_v4();
    let kinds_json = serde_json::to_string(&new.event_kinds)?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_webhook_subscriptions
            (id, workspace_id, url, label, event_kinds, secret_ciphertext)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING {SUB_COLS}"
    ))
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.url)
    .bind(&new.label)
    .bind(&kinds_json)
    .bind(&new.secret_ciphertext)
    .fetch_one(pool)
    .await?;
    row_to_subscription(&row)
}

pub async fn get(
    pool: &SqlitePool,
    id: WebhookSubscriptionId,
) -> Result<WebhookSubscriptionRow, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {SUB_COLS} FROM maidan_webhook_subscriptions WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    let subscription = row_to_subscription(&row)?;
    Ok(WebhookSubscriptionRow {
        secret_ciphertext: row.get("secret_ciphertext"),
        subscription,
    })
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<WebhookSubscription>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {SUB_COLS}
         FROM maidan_webhook_subscriptions
         WHERE workspace_id = ? AND revoked_at IS NULL
         ORDER BY created_at ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_subscription).collect()
}

pub async fn list_enabled(pool: &SqlitePool) -> Result<Vec<WebhookSubscriptionRow>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {SUB_COLS}
         FROM maidan_webhook_subscriptions
         WHERE enabled = 1 AND revoked_at IS NULL"
    ))
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let subscription = row_to_subscription(row)?;
            Ok(WebhookSubscriptionRow {
                secret_ciphertext: row.get("secret_ciphertext"),
                subscription,
            })
        })
        .collect()
}

pub async fn revoke(
    pool: &SqlitePool,
    id: WebhookSubscriptionId,
) -> Result<WebhookSubscription, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "UPDATE maidan_webhook_subscriptions
         SET enabled = 0, revoked_at = ?
         WHERE id = ? AND revoked_at IS NULL
         RETURNING {SUB_COLS}"
    ))
    .bind(&now)
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_subscription(&row)
}

pub async fn enqueue_delivery(
    pool: &SqlitePool,
    subscription_id: WebhookSubscriptionId,
    log_id: i64,
    payload: &str,
) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_webhook_deliveries (subscription_id, log_id, payload)
         VALUES (?, ?, ?)
         RETURNING id",
    )
    .bind(subscription_id.0)
    .bind(log_id)
    .bind(payload)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn list_pending_deliveries(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<WebhookSubscriptionDelivery>, StoreError> {
    let now = Utc::now().to_rfc3339();
    let rows = sqlx::query(
        "SELECT d.id, d.subscription_id, d.log_id, d.payload, d.attempts
         FROM maidan_webhook_deliveries d
         WHERE d.delivered_at IS NULL
           AND d.quarantined_at IS NULL
           AND d.next_attempt_at <= ?
         ORDER BY d.id ASC
         LIMIT ?",
    )
    .bind(&now)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| WebhookSubscriptionDelivery {
            id: row.get("id"),
            subscription_id: WebhookSubscriptionId(row.get::<Uuid, _>("subscription_id")),
            log_id: row.get("log_id"),
            payload: row.get("payload"),
            attempts: row.get("attempts"),
        })
        .collect())
}

pub async fn mark_delivered(pool: &SqlitePool, delivery_id: i64) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    let updated = sqlx::query(
        "UPDATE maidan_webhook_deliveries
         SET delivered_at = ?
         WHERE id = ? AND delivered_at IS NULL",
    )
    .bind(&now)
    .bind(delivery_id)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn record_delivery_attempt(
    pool: &SqlitePool,
    delivery_id: i64,
    error: &str,
    next_attempt_at: DateTime<Utc>,
) -> Result<i32, StoreError> {
    let next = next_attempt_at.to_rfc3339();
    let row = sqlx::query(
        "UPDATE maidan_webhook_deliveries
         SET attempts = attempts + 1,
             last_error = ?,
             next_attempt_at = ?
         WHERE id = ?
         RETURNING attempts",
    )
    .bind(error)
    .bind(&next)
    .bind(delivery_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    Ok(row.get("attempts"))
}

pub async fn quarantine_delivery(pool: &SqlitePool, delivery_id: i64) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE maidan_webhook_deliveries
         SET quarantined_at = ?
         WHERE id = ? AND quarantined_at IS NULL",
    )
    .bind(&now)
    .bind(delivery_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_subscription(row: &sqlx::sqlite::SqliteRow) -> Result<WebhookSubscription, StoreError> {
    let kinds_json: String = row.get("event_kinds");
    let event_kinds: Vec<String> = serde_json::from_str(&kinds_json)?;
    let enabled: i64 = row.get("enabled");
    Ok(WebhookSubscription {
        id: WebhookSubscriptionId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        url: row.get("url"),
        label: row.get("label"),
        event_kinds,
        enabled: enabled != 0,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        revoked_at: row.get("revoked_at"),
    })
}
