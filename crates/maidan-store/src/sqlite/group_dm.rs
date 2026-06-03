use chrono::{DateTime, Utc};
use maidan_types::{
    GroupDmConversation, GroupDmConversationId, MemberId, NewThread, ThreadId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::group_dm::group_dm_conversation_id;
use crate::sqlite::{dm, threads};

pub async fn open(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_ids: &[MemberId],
    title: Option<String>,
) -> Result<GroupDmConversation, StoreError> {
    if member_ids.len() < 3 {
        return Err(StoreError::InvalidInput(
            "group DM requires at least three members".into(),
        ));
    }
    let mut unique = member_ids.to_vec();
    unique.sort_by_key(|m| m.0);
    unique.dedup();
    if unique.len() != member_ids.len() {
        return Err(StoreError::InvalidInput(
            "group DM member_ids must be unique".into(),
        ));
    }
    for mid in &unique {
        let member = crate::sqlite::members::get(pool, *mid).await?;
        if member.workspace_id != workspace_id {
            return Err(StoreError::InvalidInput(
                "all members must belong to the workspace".into(),
            ));
        }
    }
    let channel = dm::ensure_dm_channel(pool, workspace_id).await?;
    let thread = threads::create(
        pool,
        NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: title.clone(),
        },
    )
    .await?;
    let id = group_dm_conversation_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_group_dm_conversations
            (id, workspace_id, thread_id, title, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.0)
    .bind(workspace_id.0)
    .bind(thread.id.0)
    .bind(title.as_deref())
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    for mid in &unique {
        sqlx::query(
            "INSERT INTO maidan_group_dm_members (group_dm_id, member_id, created_at)
             VALUES (?, ?, ?)",
        )
        .bind(id.0)
        .bind(mid.0)
        .bind(&now)
        .execute(pool)
        .await?;
    }
    get(pool, id).await
}

pub async fn get(
    pool: &SqlitePool,
    id: GroupDmConversationId,
) -> Result<GroupDmConversation, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, thread_id, title, created_at, updated_at
         FROM maidan_group_dm_conversations WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_group_dm(pool, &row).await
}

pub async fn list_for_member(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<GroupDmConversation>, StoreError> {
    let rows = sqlx::query(
        "SELECT g.id, g.workspace_id, g.thread_id, g.title, g.created_at, g.updated_at
         FROM maidan_group_dm_conversations g
         INNER JOIN maidan_group_dm_members m ON m.group_dm_id = g.id
         WHERE g.workspace_id = ? AND m.member_id = ?
         ORDER BY g.updated_at DESC",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_group_dm(pool, &row).await?);
    }
    Ok(out)
}

pub async fn get_for_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<GroupDmConversation>, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, thread_id, title, created_at, updated_at
         FROM maidan_group_dm_conversations WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(Some(row_to_group_dm(pool, &r).await?)),
        None => Ok(None),
    }
}

pub async fn is_member(
    pool: &SqlitePool,
    id: GroupDmConversationId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let found = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM maidan_group_dm_members WHERE group_dm_id = ? AND member_id = ? LIMIT 1",
    )
    .bind(id.0)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

async fn row_to_group_dm(
    pool: &SqlitePool,
    row: &sqlx::sqlite::SqliteRow,
) -> Result<GroupDmConversation, StoreError> {
    let id = GroupDmConversationId(row.get::<Uuid, _>("id"));
    let member_rows =
        sqlx::query("SELECT member_id FROM maidan_group_dm_members WHERE group_dm_id = ?")
            .bind(id.0)
            .fetch_all(pool)
            .await?;
    let member_ids = member_rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect();
    let created = row.get::<String, _>("created_at");
    let updated = row.get::<String, _>("updated_at");
    Ok(GroupDmConversation {
        id,
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        title: row.get("title"),
        member_ids,
        created_at: DateTime::parse_from_rfc3339(&created)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        updated_at: DateTime::parse_from_rfc3339(&updated)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}
