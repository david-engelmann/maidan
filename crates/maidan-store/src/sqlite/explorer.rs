//! Tombstone explorer, message backlinks, and EventKind census.

use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, Event, EventKind, KindCensus, KindCount, MemberId, MessageBacklinks, MessageId, Pin,
    RefSide, ThreadId, TombstoneEntityKind, TombstoneRecord, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::{messages, reactions, refs, votes};
use crate::thread_access::{readable_row, readable_thread};

pub async fn list_tombstones(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    include_purged: bool,
    limit: i64,
    reader_id: Option<MemberId>,
) -> Result<Vec<TombstoneRecord>, StoreError> {
    let limit = limit.max(1);
    let mut retained =
        list_retained(pool, workspace_id, channel_id, thread_id, limit, reader_id).await?;
    if !include_purged {
        return Ok(retained);
    }
    let purged = list_purged(pool, workspace_id, channel_id, thread_id, limit, reader_id).await?;
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
    reader_id: Option<MemberId>,
) -> Result<Vec<TombstoneRecord>, StoreError> {
    // Numbered placeholders throughout: `readable_row`'s member placeholder
    // appears three times in the generated SQL but binds once.
    let mut sql = String::from(
        "SELECT m.id, m.thread_id, m.author_id, m.tombstoned_at,
                t.channel_id, c.workspace_id
         FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?1 AND m.tombstoned_at IS NOT NULL",
    );
    let mut n = 1u32;
    if channel_id.is_some() {
        n += 1;
        sql.push_str(&format!(" AND t.channel_id = ?{n}"));
    }
    if thread_id.is_some() {
        n += 1;
        sql.push_str(&format!(" AND m.thread_id = ?{n}"));
    }
    // Filtered in the query so a tombstone on a thread the reader cannot
    // read never crowds out the limit; a NULL reader bind keeps every row.
    let readable = readable_row(
        "m.thread_id",
        "c.workspace_id",
        &format!("?{}", n + 1),
        &format!("?{}", n + 2),
    );
    sql.push_str(&format!(" AND ({readable})"));
    sql.push_str(&format!(
        " ORDER BY m.tombstoned_at DESC, m.id DESC LIMIT ?{}",
        n + 3
    ));

    let mut q = sqlx::query(&sql).bind(workspace_id.0);
    if let Some(cid) = channel_id {
        q = q.bind(cid.0);
    }
    if let Some(tid) = thread_id {
        q = q.bind(tid.0);
    }
    q = q.bind(reader_id.map(|id| id.0));
    q = q.bind(DM_CHANNEL_NAME);
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

/// The purged half of the explorer: tombstone records reconstructed from the
/// event log, for messages whose rows were hard-deleted.
///
/// `limit` is applied **in the query**. This used to materialize every
/// `message_tombstoned` event in the scope and parse each payload, only for the
/// caller to keep `limit` of them — so a heavily-purged workspace paid a full
/// scan and a full parse for a `limit=10` read.
///
/// Taking the top `limit` from each side and merging is correct: the true top
/// `limit` of a union is contained in the union of each side's top `limit`. The
/// caller re-sorts and truncates, so ordering here only has to agree on the
/// primary key (`occurred_at` descending) — and the dedup against retained rows
/// removes duplicates, which never added to the count.
async fn list_purged(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    limit: i64,
    reader_id: Option<MemberId>,
) -> Result<Vec<TombstoneRecord>, StoreError> {
    let mut sql = String::from(
        "SELECT id, payload, occurred_at, channel_id, thread_id
         FROM maidan_events
         WHERE workspace_id = ?1 AND kind = 'message_tombstoned'",
    );
    let mut n = 1u32;
    if channel_id.is_some() {
        n += 1;
        sql.push_str(&format!(" AND channel_id = ?{n}"));
    }
    if thread_id.is_some() {
        n += 1;
        sql.push_str(&format!(" AND thread_id = ?{n}"));
    }
    // Same in-query read filter as the retained half: the event's thread
    // decides, and a NULL reader bind keeps every row.
    let readable = readable_row(
        "thread_id",
        "?1",
        &format!("?{}", n + 1),
        &format!("?{}", n + 2),
    );
    sql.push_str(&format!(" AND ({readable})"));
    sql.push_str(&format!(
        " ORDER BY occurred_at DESC, id DESC LIMIT ?{}",
        n + 3
    ));
    let mut q = sqlx::query(&sql).bind(workspace_id.0);
    if let Some(cid) = channel_id {
        q = q.bind(cid.0);
    }
    if let Some(tid) = thread_id {
        q = q.bind(tid.0);
    }
    q = q.bind(reader_id.map(|id| id.0));
    q = q.bind(DM_CHANNEL_NAME);
    let rows = q.bind(limit).fetch_all(pool).await?;
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
    reader_id: Option<MemberId>,
) -> Result<MessageBacklinks, StoreError> {
    messages::get(pool, message_id).await?;
    let references = match reader_id {
        None => refs::list_to(pool, RefSide::Message, message_id.0).await?,
        Some(reader) => {
            // An unreadable target answers empty rather than leaking that
            // the message exists and is pointed at; a readable target
            // narrows its references to sources the reader may read.
            let Some((workspace_id, readable)) =
                target_read_access(pool, message_id, reader).await?
            else {
                return Ok(empty_backlinks(message_id));
            };
            if !readable {
                return Ok(empty_backlinks(message_id));
            }
            refs::list_to_for_reader(pool, RefSide::Message, message_id.0, workspace_id, reader)
                .await?
        }
    };
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

fn empty_backlinks(message_id: MessageId) -> MessageBacklinks {
    MessageBacklinks {
        message_id,
        references: Vec::new(),
        pins: Vec::new(),
        reactions: Vec::new(),
        votes: Vec::new(),
    }
}

/// The message's workspace and whether `reader_id` may read its thread, in
/// one query. `None` when the message does not exist.
async fn target_read_access(
    pool: &SqlitePool,
    message_id: MessageId,
    reader_id: MemberId,
) -> Result<Option<(WorkspaceId, bool)>, StoreError> {
    let readable = readable_thread("m.thread_id", "c.workspace_id", "?2", "?3");
    let row = sqlx::query(&format!(
        "SELECT c.workspace_id AS workspace_id, ({readable}) AS readable
         FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE m.id = ?1"
    ))
    .bind(message_id.0)
    .bind(reader_id.0)
    .bind(DM_CHANNEL_NAME)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (
            WorkspaceId(r.get::<Uuid, _>("workspace_id")),
            r.get::<bool, _>("readable"),
        )
    }))
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
    reader_id: Option<MemberId>,
) -> Result<KindCensus, StoreError> {
    let mut sql = String::from(
        "SELECT kind, COUNT(*) AS count
         FROM maidan_events
         WHERE workspace_id = ?1",
    );
    let mut n = 1u32;
    if channel_id.is_some() {
        n += 1;
        sql.push_str(&format!(" AND channel_id = ?{n}"));
    }
    if thread_id.is_some() {
        n += 1;
        sql.push_str(&format!(" AND thread_id = ?{n}"));
    }
    if !deny_channels.is_empty() {
        let mut placeholders = Vec::with_capacity(deny_channels.len());
        for _ in deny_channels {
            n += 1;
            placeholders.push(format!("?{n}"));
        }
        sql.push_str(&format!(
            " AND (channel_id IS NULL OR channel_id NOT IN ({}))",
            placeholders.join(", ")
        ));
    }
    // Counts cover only events on threads the reader may read (thread-less
    // events are the workspace's own); a NULL reader bind counts everything.
    let readable = readable_row(
        "thread_id",
        "?1",
        &format!("?{}", n + 1),
        &format!("?{}", n + 2),
    );
    sql.push_str(&format!(" AND ({readable})"));
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
    q = q.bind(reader_id.map(|id| id.0));
    q = q.bind(DM_CHANNEL_NAME);
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
