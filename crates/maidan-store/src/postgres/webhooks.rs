use chrono::{DateTime, Utc};
use maidan_types::{
    NewWebhookSubscription, WebhookDelivery, WebhookSubscription, WebhookSubscriptionDelivery,
    WebhookSubscriptionId, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::automation_deliveries::AutomationDeliveryFilter;
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
    let id = Uuid::now_v7();
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

pub async fn list_enabled_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<WebhookSubscriptionRow>, StoreError> {
    // Uses idx_webhook_subs_workspace instead of scanning every workspace's
    // enabled subscriptions on each event.
    let rows = sqlx::query(&format!(
        "SELECT {SUB_COLS}
         FROM maidan_webhook_subscriptions
         WHERE enabled = TRUE AND revoked_at IS NULL AND workspace_id = $1"
    ))
    .bind(workspace_id.0)
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
    workspace_id: WorkspaceId,
    id: WebhookSubscriptionId,
) -> Result<WebhookSubscription, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_webhook_subscriptions
         SET enabled = FALSE, revoked_at = NOW()
         WHERE id = $1 AND workspace_id = $2 AND revoked_at IS NULL
         RETURNING {SUB_COLS}"
    ))
    .bind(id.0)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_subscription(&row)
}

/// Enqueue one delivery of event `log_id` to a subscription, unless one is
/// already queued: every replica's worker sees every event, and a lag resume
/// sees it again. `None` when the delivery was already there.
///
/// The row's `created_at` is the event's `occurred_at`, so every replica
/// writes the same partition key for the same event and the unique
/// `(subscription_id, log_id, created_at)` holds across partitions (0234).
/// An id with no event behind it (a test's) falls back to now.
pub async fn enqueue_delivery(
    pool: &PgPool,
    subscription_id: WebhookSubscriptionId,
    log_id: i64,
    payload: &str,
) -> Result<Option<i64>, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_webhook_deliveries
            (subscription_id, log_id, payload, traceparent, tracestate, created_at)
         SELECT $1, $2, $3, e.traceparent, e.tracestate, COALESCE(e.occurred_at, NOW())
         FROM (SELECT 1) AS one
         LEFT JOIN maidan_events e ON e.id = $2
         ON CONFLICT (subscription_id, log_id, created_at) DO NOTHING
         RETURNING id",
    )
    .bind(subscription_id.0)
    .bind(log_id)
    .bind(payload)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| row.get("id")))
}

const PENDING: &str = "delivered_at IS NULL AND quarantined_at IS NULL";

pub async fn list_pending_deliveries(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<WebhookSubscriptionDelivery>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT d.id, d.subscription_id, d.log_id, d.payload, d.attempts, d.traceparent, d.tracestate
         FROM maidan_webhook_deliveries d
         WHERE {PENDING} AND d.next_attempt_at <= NOW()
         ORDER BY d.id ASC
         LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_pending).collect())
}

/// Claim the oldest due delivery: lease it `lease_secs` forward, so no other
/// replica's poller takes it while this one sends. Whatever the send does next
/// (delivered, a recorded attempt, a deferral, quarantine) replaces the lease;
/// a poller that dies mid-send leaves the row due again when it runs out.
pub async fn claim_next_delivery(
    pool: &PgPool,
    lease_secs: i64,
) -> Result<Option<WebhookSubscriptionDelivery>, StoreError> {
    let row = sqlx::query(&format!(
        "WITH due AS (
             SELECT id, created_at FROM maidan_webhook_deliveries
             WHERE {PENDING} AND next_attempt_at <= NOW()
             ORDER BY id ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_webhook_deliveries d
         SET next_attempt_at = NOW() + make_interval(secs => $1)
         FROM due
         WHERE d.id = due.id AND d.created_at = due.created_at
         RETURNING d.id, d.subscription_id, d.log_id, d.payload, d.attempts, d.traceparent, d.tracestate"
    ))
    .bind(lease_secs as f64)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_pending))
}

fn row_to_pending(row: &sqlx::postgres::PgRow) -> WebhookSubscriptionDelivery {
    WebhookSubscriptionDelivery {
        id: row.get("id"),
        subscription_id: WebhookSubscriptionId(row.get("subscription_id")),
        log_id: row.get("log_id"),
        payload: row.get("payload"),
        attempts: row.get("attempts"),
        trace: maidan_types::TraceContext::from_columns(
            row.try_get("traceparent").unwrap_or(None),
            row.try_get("tracestate").unwrap_or(None),
        ),
    }
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

/// Move a pending delivery's next attempt without counting one.
pub async fn defer_delivery(
    pool: &PgPool,
    delivery_id: i64,
    next_attempt_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(&format!(
        "UPDATE maidan_webhook_deliveries
         SET next_attempt_at = $2
         WHERE id = $1 AND {PENDING}"
    ))
    .bind(delivery_id)
    .bind(next_attempt_at)
    .execute(pool)
    .await?;
    Ok(())
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

pub async fn list_deliveries_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    filter: AutomationDeliveryFilter,
    limit: i64,
) -> Result<Vec<WebhookDelivery>, StoreError> {
    let predicate = match filter {
        AutomationDeliveryFilter::Pending => PENDING,
        AutomationDeliveryFilter::DeadLetter => "d.quarantined_at IS NOT NULL",
        AutomationDeliveryFilter::Delivered => "d.delivered_at IS NOT NULL",
    };
    let rows = sqlx::query(&format!(
        "SELECT d.id, s.workspace_id, d.subscription_id, d.log_id, s.url AS target_url,
                d.attempts, d.last_error, d.delivered_at, d.quarantined_at, d.next_attempt_at, d.created_at
         FROM maidan_webhook_deliveries d
         JOIN maidan_webhook_subscriptions s ON s.id = d.subscription_id
         WHERE s.workspace_id = $1 AND {predicate}
         ORDER BY d.id DESC
         LIMIT $2"
    ))
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_webhook_delivery).collect()
}

pub async fn get_delivery(
    pool: &PgPool,
    delivery_id: i64,
    workspace_id: WorkspaceId,
) -> Result<WebhookDelivery, StoreError> {
    let row = sqlx::query(
        "SELECT d.id, s.workspace_id, d.subscription_id, d.log_id, s.url AS target_url,
                d.attempts, d.last_error, d.delivered_at, d.quarantined_at, d.next_attempt_at, d.created_at
         FROM maidan_webhook_deliveries d
         JOIN maidan_webhook_subscriptions s ON s.id = d.subscription_id
         WHERE d.id = $1 AND s.workspace_id = $2",
    )
    .bind(delivery_id)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_webhook_delivery(&row)
}

pub async fn replay_delivery(
    pool: &PgPool,
    delivery_id: i64,
    workspace_id: WorkspaceId,
) -> Result<WebhookDelivery, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_webhook_deliveries d
         SET quarantined_at = NULL,
             delivered_at = NULL,
             next_attempt_at = NOW(),
             attempts = 0,
             last_error = NULL
         FROM maidan_webhook_subscriptions s
         WHERE d.subscription_id = s.id
           AND d.id = $1
           AND s.workspace_id = $2
           AND d.quarantined_at IS NOT NULL
         RETURNING d.id, s.workspace_id, d.subscription_id, d.log_id, s.url AS target_url,
                   d.attempts, d.last_error, d.delivered_at, d.quarantined_at, d.next_attempt_at, d.created_at",
    )
    .bind(delivery_id)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_webhook_delivery(&row)
}

fn row_to_webhook_delivery(row: &sqlx::postgres::PgRow) -> Result<WebhookDelivery, StoreError> {
    Ok(WebhookDelivery {
        id: row.get("id"),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        subscription_id: WebhookSubscriptionId(row.get("subscription_id")),
        log_id: row.get("log_id"),
        target_url: row.get("target_url"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        delivered_at: row.get("delivered_at"),
        quarantined_at: row.get("quarantined_at"),
        next_attempt_at: row.get::<DateTime<Utc>, _>("next_attempt_at"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
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
