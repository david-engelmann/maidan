use chrono::{DateTime, Utc};
use maidan_types::{Channel, ChannelId, NewChannel, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewChannel) -> Result<Channel, StoreError> {
    let id = Uuid::new_v4();
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
         FROM maidan_channels WHERE workspace_id = ? ORDER BY name ASC",
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
