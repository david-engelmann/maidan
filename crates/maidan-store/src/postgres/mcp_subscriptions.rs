//! Postgres-backed stateless MCP resource subscriptions. See
//! `crate::mcp_subscriptions`.

use chrono::{DateTime, Utc};
use maidan_types::WorkspaceId;
use sqlx::PgPool;

use crate::error::StoreError;
use crate::mcp_subscriptions::{McpSubscriptionWatch, NewMcpSubscription};

pub async fn subscribe(
    pool: &PgPool,
    new: &NewMcpSubscription,
    limit: usize,
) -> Result<bool, StoreError> {
    let now = Utc::now();
    sqlx::query(
        "DELETE FROM maidan_mcp_resource_subscriptions
          WHERE subscriber = $1 AND expires_at <= $2",
    )
    .bind(&new.subscriber)
    .bind(now)
    .execute(pool)
    .await?;
    // Re-subscribing to what is already watched is not growth, so the limit
    // only binds a new row.
    let taken: Option<(i32,)> = sqlx::query_as(
        "INSERT INTO maidan_mcp_resource_subscriptions
             (subscriber, uri, workspace_id, member_id, expires_at)
         SELECT $1, $2, $3, $4, $5
          WHERE EXISTS (SELECT 1 FROM maidan_mcp_resource_subscriptions
                         WHERE subscriber = $1 AND uri = $2)
             OR (SELECT COUNT(*) FROM maidan_mcp_resource_subscriptions
                  WHERE subscriber = $1) < $6
         ON CONFLICT (subscriber, uri) DO UPDATE SET expires_at = EXCLUDED.expires_at
         RETURNING 1",
    )
    .bind(&new.subscriber)
    .bind(&new.uri)
    .bind(new.workspace_id.map(|w| w.0))
    .bind(new.member_id.map(|m| m.0))
    .bind(new.expires_at)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_optional(pool)
    .await?;
    if taken.is_none() {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE maidan_mcp_resource_subscriptions SET expires_at = $2
          WHERE subscriber = $1 AND expires_at < $2",
    )
    .bind(&new.subscriber)
    .bind(new.expires_at)
    .execute(pool)
    .await?;
    Ok(true)
}

pub async fn unsubscribe(pool: &PgPool, subscriber: &str, uri: &str) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "DELETE FROM maidan_mcp_resource_subscriptions WHERE subscriber = $1 AND uri = $2",
    )
    .bind(subscriber)
    .bind(uri)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn watchers(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    uris: &[String],
    subscribers: &[String],
    now: DateTime<Utc>,
) -> Result<Vec<McpSubscriptionWatch>, StoreError> {
    if uris.is_empty() || subscribers.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT subscriber, uri FROM maidan_mcp_resource_subscriptions
          WHERE uri = ANY($1) AND subscriber = ANY($2)
            AND (workspace_id = $3 OR workspace_id IS NULL)
            AND expires_at > $4",
    )
    .bind(uris)
    .bind(subscribers)
    .bind(workspace_id.0)
    .bind(now)
    .fetch_all(pool)
    .await?)
}

pub async fn extend(
    pool: &PgPool,
    subscribers: &[String],
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<u64, StoreError> {
    if subscribers.is_empty() {
        return Ok(0);
    }
    let done = sqlx::query(
        "UPDATE maidan_mcp_resource_subscriptions SET expires_at = $3
          WHERE subscriber = ANY($1) AND expires_at > $2 AND expires_at < $3",
    )
    .bind(subscribers)
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

pub async fn reap(pool: &PgPool, now: DateTime<Utc>) -> Result<u64, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_mcp_resource_subscriptions WHERE expires_at <= $1")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(done.rows_affected())
}
