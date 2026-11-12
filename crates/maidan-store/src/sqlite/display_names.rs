//! Names for search hits: channel names, thread titles and member handles
//! for a set of ids in one workspace, a few queries per id chunk.

use maidan_types::{ChannelId, MemberId, ThreadId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::DisplayNames;

/// Ids per `IN (...)` list, well under SQLite's bound-parameter limit.
const SQLITE_IN_CHUNK: usize = 400;

pub async fn load(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    channels: &[ChannelId],
    threads: &[ThreadId],
    members: &[MemberId],
) -> Result<DisplayNames, StoreError> {
    let mut names = DisplayNames::default();
    let channel_ids: Vec<Uuid> = channels.iter().map(|id| id.0).collect();
    let thread_ids: Vec<Uuid> = threads.iter().map(|id| id.0).collect();
    let member_ids: Vec<Uuid> = members.iter().map(|id| id.0).collect();
    for (id, name) in named(
        pool,
        workspace_id,
        &channel_ids,
        "SELECT c.id, c.name FROM maidan_channels c WHERE c.workspace_id = ? AND c.id IN",
    )
    .await?
    {
        names.channels.insert(ChannelId(id), name);
    }
    // A thread is in the workspace through its channel.
    for (id, name) in named(
        pool,
        workspace_id,
        &thread_ids,
        "SELECT t.id, t.title AS name FROM maidan_threads t
           JOIN maidan_channels c ON c.id = t.channel_id
          WHERE c.workspace_id = ? AND t.title IS NOT NULL AND t.id IN",
    )
    .await?
    {
        names.threads.insert(ThreadId(id), name);
    }
    for (id, name) in named(
        pool,
        workspace_id,
        &member_ids,
        "SELECT m.id, m.handle AS name FROM maidan_members m WHERE m.workspace_id = ? AND m.id IN",
    )
    .await?
    {
        names.members.insert(MemberId(id), name);
    }
    Ok(names)
}

/// `(id, name)` for each id in `ids` that `select` (ending in `IN`) finds.
async fn named(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    ids: &[Uuid],
    select: &str,
) -> Result<Vec<(Uuid, String)>, StoreError> {
    let mut out = Vec::new();
    for chunk in ids.chunks(SQLITE_IN_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!("{select} ({placeholders})");
        let mut query = sqlx::query(&sql).bind(workspace_id.0);
        for id in chunk {
            query = query.bind(*id);
        }
        for row in query.fetch_all(pool).await? {
            out.push((row.get("id"), row.get("name")));
        }
    }
    Ok(out)
}
