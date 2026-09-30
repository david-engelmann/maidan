//! SQLite-backed stateless MCP resource subscriptions. See
//! `crate::mcp_subscriptions`. Times are millisecond `...Z` text, so string
//! comparison is time order; lists are bound as one JSON array.

use chrono::{DateTime, SecondsFormat, Utc};
use maidan_types::WorkspaceId;
use sqlx::SqlitePool;

use crate::error::StoreError;
use crate::mcp_subscriptions::{McpSubscriptionWatch, NewMcpSubscription};

fn ms(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn json_list(items: &[String]) -> Result<String, StoreError> {
    Ok(serde_json::to_string(items)?)
}

pub async fn subscribe(
    pool: &SqlitePool,
    new: &NewMcpSubscription,
    limit: usize,
) -> Result<bool, StoreError> {
    let now = ms(Utc::now());
    let expires_at = ms(new.expires_at);
    sqlx::query(
        "DELETE FROM maidan_mcp_resource_subscriptions
          WHERE subscriber = ?1 AND expires_at <= ?2",
    )
    .bind(&new.subscriber)
    .bind(&now)
    .execute(pool)
    .await?;
    // Re-subscribing to what is already watched is not growth, so the limit
    // only binds a new row.
    let taken: Option<(i32,)> = sqlx::query_as(
        "INSERT INTO maidan_mcp_resource_subscriptions
             (subscriber, uri, workspace_id, member_id, expires_at)
         SELECT ?1, ?2, ?3, ?4, ?5
          WHERE EXISTS (SELECT 1 FROM maidan_mcp_resource_subscriptions
                         WHERE subscriber = ?1 AND uri = ?2)
             OR (SELECT COUNT(*) FROM maidan_mcp_resource_subscriptions
                  WHERE subscriber = ?1) < ?6
         ON CONFLICT (subscriber, uri) DO UPDATE SET expires_at = excluded.expires_at
         RETURNING 1",
    )
    .bind(&new.subscriber)
    .bind(&new.uri)
    .bind(new.workspace_id.map(|w| w.0))
    .bind(new.member_id.map(|m| m.0))
    .bind(&expires_at)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_optional(pool)
    .await?;
    if taken.is_none() {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE maidan_mcp_resource_subscriptions SET expires_at = ?2
          WHERE subscriber = ?1 AND expires_at < ?2",
    )
    .bind(&new.subscriber)
    .bind(&expires_at)
    .execute(pool)
    .await?;
    Ok(true)
}

pub async fn unsubscribe(
    pool: &SqlitePool,
    subscriber: &str,
    uri: &str,
) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "DELETE FROM maidan_mcp_resource_subscriptions WHERE subscriber = ?1 AND uri = ?2",
    )
    .bind(subscriber)
    .bind(uri)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn watchers(
    pool: &SqlitePool,
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
          WHERE uri IN (SELECT value FROM json_each(?1))
            AND subscriber IN (SELECT value FROM json_each(?2))
            AND (workspace_id = ?3 OR workspace_id IS NULL)
            AND expires_at > ?4",
    )
    .bind(json_list(uris)?)
    .bind(json_list(subscribers)?)
    .bind(workspace_id.0)
    .bind(ms(now))
    .fetch_all(pool)
    .await?)
}

pub async fn extend(
    pool: &SqlitePool,
    subscribers: &[String],
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<u64, StoreError> {
    if subscribers.is_empty() {
        return Ok(0);
    }
    let expires_at = ms(expires_at);
    let done = sqlx::query(
        "UPDATE maidan_mcp_resource_subscriptions SET expires_at = ?3
          WHERE subscriber IN (SELECT value FROM json_each(?1))
            AND expires_at > ?2 AND expires_at < ?3",
    )
    .bind(json_list(subscribers)?)
    .bind(ms(now))
    .bind(&expires_at)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

pub async fn reap(pool: &SqlitePool, now: DateTime<Utc>) -> Result<u64, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_mcp_resource_subscriptions WHERE expires_at <= ?1")
        .bind(ms(now))
        .execute(pool)
        .await?;
    Ok(done.rows_affected())
}
