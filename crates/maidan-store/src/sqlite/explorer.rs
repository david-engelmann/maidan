//! Tombstone explorer, message backlinks, and EventKind census (Cluster 394).

use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, Event, EventKind, KindCensus, KindCount, MessageBacklinks, MessageId, Pin, RefSide,
    ThreadId, TombstoneEntityKind, TombstoneRecord, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::{messages, reactions, refs, votes};

pub async fn list_tombstones(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    include_purged: bool,
    limit: i64,
) -> Result<Vec<TombstoneRecord>, StoreError> {
    let limit = limit.max(1);
    let mut retained = list_retained(pool, workspace_id, channel_id, thread_id, limit).await?;
    if !include_purged {
        return Ok(retained);
    }
    let purged = list_purged(pool, workspace_id, channel_id, thread_id).await?;
    let retained_ids: std::collections::HashSet<Uuid> = retained.iter().map(|r| r.id).collect();
    for row in purged {
        if !retained_ids.contains(&row.id) {
            retained.push(row);
        }
    }
    retained.sort_by(|a, b| b.tombstoned_at.cmp(&a.tombstoned_at).then(b.id.cmp(&a.id)));
    retained.truncate(limit as usize);
    Ok(retained)
}

async fn list_retained(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<TombstoneRecord>, StoreError> {
    let mut sql = String::from(
        "SELECT m.id, m.thread_id, m.author_id, m.tombstoned_at,
                t.channel_id, c.workspace_id
         FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ? AND m.tombstoned_at IS NOT NULL",
    );
    if channel_id.is_some() {
        sql.push_str(" AND t.channel_id = ?");
    }
    if thread_id.is_some() {
        sql.push_str(" AND m.thread_id = ?");
    }
    sql.push_str(" ORDER BY m.tombstoned_at DESC, m.id DESC LIMIT ?");

    let mut q = sqlx::query(&sql).bind(workspace_id.0);
    if let Some(cid) = channel_id {
        q = q.bind(cid.0);
    }
    if let Some(tid) = thread_id {
        q = q.bind(tid.0);
    }
    q = q.bind(limit);
    let rows = q.fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_retained).collect())
}

fn row_to_retained(row: &sqlx::sqlite::SqliteRow) -> TombstoneRecord {
    TombstoneRecord {
        entity_kind: TombstoneEntityKind::Message,
        id: row.get::<Uuid, _>("id"),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        author_id: Some(maidan_types::MemberId(row.get::<Uuid, _>("author_id"))),
        tombstoned_at: row.get::<DateTime<Utc>, _>("tombstoned_at"),
        retained: true,
        source_log_id: None,
    }
}

async fn list_purged(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
) -> Result<Vec<TombstoneRecord>, StoreError> {
    let mut sql = String::from(
        "SELECT id, payload, occurred_at, channel_id, thread_id
         FROM maidan_events
         WHERE workspace_id = ? AND kind = 'message_tombstoned'",
    );
    if channel_id.is_some() {
        sql.push_str(" AND channel_id = ?");
    }
    if thread_id.is_some() {
        sql.push_str(" AND thread_id = ?");
    }
    let mut q = sqlx::query(&sql).bind(workspace_id.0);
    if let Some(cid) = channel_id {
        q = q.bind(cid.0);
    }
    if let Some(tid) = thread_id {
        q = q.bind(tid.0);
    }
    let rows = q.fetch_all(pool).await?;
    let mut out = Vec::new();
    for row in &rows {
        let payload_text: String = row.get("payload");
        let Ok(event) = serde_json::from_str::<Event>(&payload_text) else {
            continue;
        };
        let Event::MessageTombstoned {
            message_id,
            channel_id: ev_ch,
            thread_id: ev_th,
            occurred_at,
            ..
        } = event
        else {
            continue;
        };
        out.push(TombstoneRecord {
            entity_kind: TombstoneEntityKind::Message,
            id: message_id.0,
            workspace_id,
            channel_id: ev_ch,
            thread_id: ev_th,
            author_id: None,
            tombstoned_at: occurred_at,
            retained: false,
            source_log_id: Some(row.get::<i64, _>("id")),
        });
    }
    Ok(out)
}

pub async fn list_message_backlinks(
    pool: &SqlitePool,
    message_id: MessageId,
) -> Result<MessageBacklinks, StoreError> {
    messages::get(pool, message_id).await?;
    let references = refs::list_to(pool, RefSide::Message, message_id.0).await?;
    let pins = list_pins_for_message(pool, message_id).await?;
    let reactions = reactions::list(pool, message_id).await?;
    let votes = votes::list(pool, message_id).await?;
    Ok(MessageBacklinks {
        message_id,
        references,
        pins,
        reactions,
        votes,
    })
}

async fn list_pins_for_message(
    pool: &SqlitePool,
    message_id: MessageId,
) -> Result<Vec<Pin>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, message_id, member_id, created_at
         FROM maidan_pins
         WHERE message_id = ?
         ORDER BY created_at ASC",
    )
    .bind(message_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Pin {
            thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
            message_id: MessageId(row.get::<Uuid, _>("message_id")),
            member_id: maidan_types::MemberId(row.get::<Uuid, _>("member_id")),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}

pub async fn event_kind_census(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    deny_channels: &[ChannelId],
) -> Result<KindCensus, StoreError> {
    let mut sql = String::from(
        "SELECT kind, COUNT(*) AS count
         FROM maidan_events
         WHERE workspace_id = ?",
    );
    if channel_id.is_some() {
        sql.push_str(" AND channel_id = ?");
    }
    if thread_id.is_some() {
        sql.push_str(" AND thread_id = ?");
    }
    if !deny_channels.is_empty() {
        let placeholders = vec!["?"; deny_channels.len()].join(", ");
        sql.push_str(&format!(
            " AND (channel_id IS NULL OR channel_id NOT IN ({placeholders}))"
        ));
    }
    sql.push_str(" GROUP BY kind ORDER BY count DESC, kind ASC");

    let mut q = sqlx::query(&sql).bind(workspace_id.0);
    if let Some(cid) = channel_id {
        q = q.bind(cid.0);
    }
    if let Some(tid) = thread_id {
        q = q.bind(tid.0);
    }
    for cid in deny_channels {
        q = q.bind(cid.0);
    }
    let rows = q.fetch_all(pool).await?;
    let mut counts = Vec::new();
    let mut total = 0_i64;
    for row in &rows {
        let raw: String = row.get("kind");
        let Some(kind) = EventKind::parse(&raw) else {
            continue;
        };
        let count: i64 = row.get("count");
        total += count;
        counts.push(KindCount { kind, count });
    }
    Ok(KindCensus {
        workspace_id,
        channel_id,
        thread_id,
        total,
        counts,
    })
}
