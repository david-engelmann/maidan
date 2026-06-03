use chrono::Utc;
use maidan_types::{
    GroupDmConversation, GroupDmConversationId, MemberId, NewThread, ThreadId, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::group_dm::group_dm_conversation_id;
use crate::postgres::{dm, threads};

pub async fn open(
    pool: &PgPool,
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
        let member = crate::postgres::members::get(pool, *mid).await?;
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
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_group_dm_conversations
            (id, workspace_id, thread_id, title, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $5)",
    )
    .bind(id.0)
    .bind(workspace_id.0)
    .bind(thread.id.0)
    .bind(title.as_deref())
    .bind(now)
    .execute(pool)
    .await?;
    for mid in &unique {
        sqlx::query(
            "INSERT INTO maidan_group_dm_members (group_dm_id, member_id, created_at)
             VALUES ($1, $2, $3)",
        )
        .bind(id.0)
        .bind(mid.0)
        .bind(now)
        .execute(pool)
        .await?;
    }
    get(pool, id).await
}

pub async fn get(
    pool: &PgPool,
    id: GroupDmConversationId,
) -> Result<GroupDmConversation, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, thread_id, title, created_at, updated_at
         FROM maidan_group_dm_conversations WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_group_dm(pool, &row).await
}

pub async fn list_for_member(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<Vec<GroupDmConversation>, StoreError> {
    let rows = sqlx::query(
        "SELECT g.id, g.workspace_id, g.thread_id, g.title, g.created_at, g.updated_at
         FROM maidan_group_dm_conversations g
         INNER JOIN maidan_group_dm_members m ON m.group_dm_id = g.id
         WHERE g.workspace_id = $1 AND m.member_id = $2
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
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Option<GroupDmConversation>, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, thread_id, title, created_at, updated_at
         FROM maidan_group_dm_conversations WHERE thread_id = $1",
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
    pool: &PgPool,
    id: GroupDmConversationId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let found = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM maidan_group_dm_members WHERE group_dm_id = $1 AND member_id = $2 LIMIT 1",
    )
    .bind(id.0)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

async fn row_to_group_dm(
    pool: &PgPool,
    row: &sqlx::postgres::PgRow,
) -> Result<GroupDmConversation, StoreError> {
    let id = GroupDmConversationId(row.get::<Uuid, _>("id"));
    let member_rows =
        sqlx::query("SELECT member_id FROM maidan_group_dm_members WHERE group_dm_id = $1")
            .bind(id.0)
            .fetch_all(pool)
            .await?;
    let member_ids = member_rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect();
    Ok(GroupDmConversation {
        id,
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        title: row.get("title"),
        member_ids,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
