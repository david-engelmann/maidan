use chrono::{DateTime, Utc};
use maidan_types::{Channel, ChannelId, Event, NewChannel, StoredEvent, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

pub async fn create(pool: &PgPool, new: NewChannel) -> Result<Channel, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_channels (id, workspace_id, name, topic, private)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(new.topic.as_deref())
    .bind(new.private)
    .fetch_one(pool)
    .await
    .map_err(map_channel_err)?;
    Ok(row_to_channel(&row))
}

/// Insert a channel and append its `ChannelCreated` event in one transaction
/// (Cluster 205 transactional outbox) — see the SQLite twin.
pub async fn create_with_event(
    pool: &PgPool,
    new: NewChannel,
) -> Result<(Channel, StoredEvent), StoreError> {
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_channels (id, workspace_id, name, topic, private)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(new.topic.as_deref())
    .bind(new.private)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_channel_err)?;
    let channel = row_to_channel(&row);
    let event = Event::ChannelCreated {
        occurred_at: Utc::now(),
        workspace_id: channel.workspace_id,
        channel: channel.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((channel, stored))
}

pub async fn get(pool: &PgPool, id: ChannelId) -> Result<Channel, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at
         FROM maidan_channels WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_channel(&row))
}

pub async fn list(pool: &PgPool, workspace_id: WorkspaceId) -> Result<Vec<Channel>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at
         FROM maidan_channels WHERE workspace_id = $1 ORDER BY name ASC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_channel).collect())
}

fn map_channel_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict("channel name already exists in workspace".into());
        }
    }
    StoreError::Database(err)
}

fn row_to_channel(row: &sqlx::postgres::PgRow) -> Channel {
    Channel {
        id: ChannelId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        name: row.get("name"),
        topic: row.get("topic"),
        private: row.get("private"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    }
}
