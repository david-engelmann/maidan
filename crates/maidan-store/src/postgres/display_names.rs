//! Names for search hits: channel names, thread titles and member handles
//! for a set of ids in one workspace, in one query.

use maidan_types::{ChannelId, MemberId, ThreadId, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::DisplayNames;

pub async fn load(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    channels: &[ChannelId],
    threads: &[ThreadId],
    members: &[MemberId],
) -> Result<DisplayNames, StoreError> {
    let mut names = DisplayNames::default();
    if channels.is_empty() && threads.is_empty() && members.is_empty() {
        return Ok(names);
    }
    let channel_ids: Vec<Uuid> = channels.iter().map(|id| id.0).collect();
    let thread_ids: Vec<Uuid> = threads.iter().map(|id| id.0).collect();
    let member_ids: Vec<Uuid> = members.iter().map(|id| id.0).collect();
    // The workspace bound sits on every arm: a thread is in the workspace
    // through its channel.
    let rows = sqlx::query(
        "SELECT 'channel' AS kind, c.id, c.name FROM maidan_channels c
          WHERE c.workspace_id = $1 AND c.id = ANY($2)
         UNION ALL
         SELECT 'thread', t.id, t.title FROM maidan_threads t
           JOIN maidan_channels c ON c.id = t.channel_id
          WHERE c.workspace_id = $1 AND t.id = ANY($3) AND t.title IS NOT NULL
         UNION ALL
         SELECT 'member', m.id, m.handle FROM maidan_members m
          WHERE m.workspace_id = $1 AND m.id = ANY($4)",
    )
    .bind(workspace_id.0)
    .bind(&channel_ids)
    .bind(&thread_ids)
    .bind(&member_ids)
    .fetch_all(pool)
    .await?;
    for row in &rows {
        let id: Uuid = row.get("id");
        let name: String = row.get("name");
        match row.get::<&str, _>("kind") {
            "channel" => names.channels.insert(ChannelId(id), name),
            "thread" => names.threads.insert(ThreadId(id), name),
            _ => names.members.insert(MemberId(id), name),
        };
    }
    Ok(names)
}
