use chrono::{DateTime, Utc};
use maidan_types::{Channel, ChannelId, Event, MemberId, NewChannel, StoredEvent, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

pub async fn create(pool: &SqlitePool, new: NewChannel) -> Result<Channel, StoreError> {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_channels (id, workspace_id, name, topic, private, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(new.topic.as_deref())
    .bind(new.private)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await
    .map_err(map_channel_err)?;
    Ok(row_to_channel(&row))
}

/// Insert a channel and append its `ChannelCreated` event in one transaction —
/// the row and the event commit atomically.
pub async fn create_with_event(
    pool: &SqlitePool,
    new: NewChannel,
) -> Result<(Channel, StoredEvent), StoreError> {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let mut tx = super::tx::begin_write(pool).await?;
    let row = sqlx::query(
        "INSERT INTO maidan_channels (id, workspace_id, name, topic, private, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(new.topic.as_deref())
    .bind(new.private)
    .bind(now)
    .bind(now)
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

pub async fn get(pool: &SqlitePool, id: ChannelId) -> Result<Channel, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at
         FROM maidan_channels WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_channel(&row))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<Channel>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at
         FROM maidan_channels WHERE workspace_id = ? ORDER BY name ASC, id ASC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_channel).collect())
}

pub async fn list_with_membership(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<(Channel, bool)>, StoreError> {
    let rows = sqlx::query(
        "SELECT c.id, c.workspace_id, c.name, c.topic, c.private, c.created_at, c.updated_at,
                c.tombstoned_at,
                EXISTS (SELECT 1 FROM maidan_channel_members m
                        WHERE m.channel_id = c.id AND m.member_id = ?2) AS is_member
         FROM maidan_channels c WHERE c.workspace_id = ?1 ORDER BY c.name ASC, c.id ASC",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row_to_channel(row), row.get::<bool, _>("is_member")))
        .collect())
}

fn map_channel_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err
        && db.is_unique_violation()
    {
        return StoreError::Conflict("channel name already exists in workspace".into());
    }
    StoreError::Database(err)
}

fn row_to_channel(row: &sqlx::sqlite::SqliteRow) -> Channel {
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
