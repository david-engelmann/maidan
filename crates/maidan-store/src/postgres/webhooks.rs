use chrono::{DateTime, Utc};
use maidan_types::{
    NewWebhookSubscription, WebhookSubscription, WebhookSubscriptionDelivery,
    WebhookSubscriptionId, WorkspaceId,
};
use sqlx::{PgPool, Row};
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
    pool: &PgPool,
    new: NewWebhookSubscription,
) -> Result<WebhookSubscription, StoreError> {
    let id = Uuid::new_v4();
    let kinds_json = serde_json::to_string(&new.event_kinds)?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_webhook_subscriptions
            (id, workspace_id, url, label, event_kinds, secret_ciphertext)
         VALUES ($1, $2, $3, $4, $5, $6)
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
    pool: &PgPool,
    id: WebhookSubscriptionId,
) -> Result<WebhookSubscriptionRow, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {SUB_COLS} FROM maidan_webhook_subscriptions WHERE id = $1"
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<WebhookSubscription>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {SUB_COLS}
         FROM maidan_webhook_subscriptions
         WHERE workspace_id = $1 AND revoked_at IS NULL
         ORDER BY created_at ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_subscription).collect()
}

pub async fn list_enabled(pool: &PgPool) -> Result<Vec<WebhookSubscriptionRow>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {SUB_COLS}
         FROM maidan_webhook_subscriptions
         WHERE enabled = TRUE AND revoked_at IS NULL"
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
    pool: &PgPool,
    id: WebhookSubscriptionId,
) -> Result<WebhookSubscription, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_webhook_subscriptions
         SET enabled = FALSE, revoked_at = NOW()
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING {SUB_COLS}"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_subscription(&row)
}

pub async fn enqueue_delivery(
    pool: &PgPool,
    subscription_id: WebhookSubscriptionId,
    log_id: i64,
    payload: &str,
) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_webhook_deliveries (subscription_id, log_id, payload)
         VALUES ($1, $2, $3)
         RETURNING id",
    )
    .bind(subscription_id.0)
    .bind(log_id)
    .bind(payload)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

const PENDING: &str = "delivered_at IS NULL AND quarantined_at IS NULL";

pub async fn list_pending_deliveries(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<WebhookSubscriptionDelivery>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT d.id, d.subscription_id, d.log_id, d.payload, d.attempts
         FROM maidan_webhook_deliveries d
         WHERE {PENDING} AND d.next_attempt_at <= NOW()
         ORDER BY d.id ASC
         LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| WebhookSubscriptionDelivery {
            id: row.get("id"),
            subscription_id: WebhookSubscriptionId(row.get("subscription_id")),
            log_id: row.get("log_id"),
            payload: row.get("payload"),
            attempts: row.get("attempts"),
        })
        .collect())
}

pub async fn mark_delivered(pool: &PgPool, delivery_id: i64) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_webhook_deliveries
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

pub async fn record_delivery_attempt(
    pool: &PgPool,
    delivery_id: i64,
    error: &str,
    next_attempt_at: DateTime<Utc>,
) -> Result<i32, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_webhook_deliveries
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

pub async fn quarantine_delivery(pool: &PgPool, delivery_id: i64) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_webhook_deliveries
         SET quarantined_at = NOW()
         WHERE id = $1 AND quarantined_at IS NULL",
    )
    .bind(delivery_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_subscription(row: &sqlx::postgres::PgRow) -> Result<WebhookSubscription, StoreError> {
    let kinds_json: String = row.get("event_kinds");
    let event_kinds: Vec<String> = serde_json::from_str(&kinds_json)?;
    Ok(WebhookSubscription {
        id: WebhookSubscriptionId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        url: row.get("url"),
        label: row.get("label"),
        event_kinds,
        enabled: row.get("enabled"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        revoked_at: row.get("revoked_at"),
    })
}
