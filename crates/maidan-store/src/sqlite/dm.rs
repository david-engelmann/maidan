use chrono::{DateTime, Utc};
use maidan_types::{
    Channel, DmConversation, DmConversationId, MemberId, NewChannel, NewThread, ThreadId,
    WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::dm::{dm_conversation_id, ordered_members};
use crate::error::StoreError;
use crate::sqlite::{channels, threads};

pub async fn ensure_dm_channel(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Channel, StoreError> {
    let existing = channels::list(pool, workspace_id).await?;
    if let Some(ch) = existing.into_iter().find(|c| c.name == DM_CHANNEL_NAME) {
        return Ok(ch);
    }
    channels::create(
        pool,
        NewChannel {
            workspace_id,
            name: DM_CHANNEL_NAME.to_string(),
            topic: Some("Direct messages".into()),
            private: true,
        },
    )
    .await
}

pub async fn get(pool: &SqlitePool, id: DmConversationId) -> Result<DmConversation, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, member_low_id, member_high_id, thread_id, created_at, updated_at
         FROM maidan_dm_conversations WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_dm(&row))
}

pub async fn get_by_members(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_a: MemberId,
    member_b: MemberId,
) -> Result<Option<DmConversation>, StoreError> {
    let (low, high) =
        ordered_members(member_a, member_b).map_err(|e| StoreError::InvalidInput(e.into()))?;
    let row = sqlx::query(
        "SELECT id, workspace_id, member_low_id, member_high_id, thread_id, created_at, updated_at
         FROM maidan_dm_conversations
         WHERE workspace_id = ? AND member_low_id = ? AND member_high_id = ?",
    )
    .bind(workspace_id.0)
    .bind(low.0)
    .bind(high.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| row_to_dm(&r)))
}

pub async fn list_for_member(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<DmConversation>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, member_low_id, member_high_id, thread_id, created_at, updated_at
         FROM maidan_dm_conversations
         WHERE workspace_id = ? AND (member_low_id = ? OR member_high_id = ?)
         ORDER BY updated_at DESC",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_dm).collect())
}

pub async fn open(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_a: MemberId,
    member_b: MemberId,
) -> Result<DmConversation, StoreError> {
    if let Some(existing) = get_by_members(pool, workspace_id, member_a, member_b).await? {
        return Ok(existing);
    }
    let (low, high) =
        ordered_members(member_a, member_b).map_err(|e| StoreError::InvalidInput(e.into()))?;
    let channel = ensure_dm_channel(pool, workspace_id).await?;
    let thread = threads::create(
        pool,
        NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        },
    )
    .await?;
    let id = dm_conversation_id();
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_dm_conversations
            (id, workspace_id, member_low_id, member_high_id, thread_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, member_low_id, member_high_id, thread_id, created_at, updated_at",
    )
    .bind(id.0)
    .bind(workspace_id.0)
    .bind(low.0)
    .bind(high.0)
    .bind(thread.id.0)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_dm(&row))
}

pub async fn get_for_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<DmConversation>, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, member_low_id, member_high_id, thread_id, created_at, updated_at
         FROM maidan_dm_conversations WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| row_to_dm(&r)))
}

fn row_to_dm(row: &sqlx::sqlite::SqliteRow) -> DmConversation {
    let created = row.get::<String, _>("created_at");
    let updated = row.get::<String, _>("updated_at");
    DmConversation {
        id: DmConversationId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        member_low_id: MemberId(row.get::<Uuid, _>("member_low_id")),
        member_high_id: MemberId(row.get::<Uuid, _>("member_high_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        created_at: DateTime::parse_from_rfc3339(&created)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        updated_at: DateTime::parse_from_rfc3339(&updated)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    }
}
