use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, InboxItem, InboxItemKind, MemberId, MemberInbox, MessageId, ThreadId,
};
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;

const EPOCH: &str = "1970-01-01T00:00:00Z";

fn parse_ts(s: &str) -> Result<DateTime<Utc>, StoreError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| StoreError::InvalidInput(format!("bad timestamp: {e}")))
}

pub async fn get_last_read_at(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<DateTime<Utc>, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT last_read_at FROM maidan_inbox_cursor WHERE member_id = ?")
            .bind(member_id.0)
            .fetch_optional(pool)
            .await?;
    match row {
        Some((s,)) => parse_ts(&s),
        None => parse_ts(EPOCH),
    }
}

pub async fn advance_last_read_at(
    pool: &SqlitePool,
    member_id: MemberId,
    read_through: DateTime<Utc>,
) -> Result<DateTime<Utc>, StoreError> {
    let existing = get_last_read_at(pool, member_id).await?;
    let new_ts = read_through.max(existing);
    let new_s = new_ts.to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_inbox_cursor (member_id, last_read_at, updated_at)
         VALUES (?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT (member_id) DO UPDATE SET
           last_read_at = CASE
             WHEN excluded.last_read_at > maidan_inbox_cursor.last_read_at
             THEN excluded.last_read_at
             ELSE maidan_inbox_cursor.last_read_at
           END,
           updated_at = CURRENT_TIMESTAMP",
    )
    .bind(member_id.0)
    .bind(&new_s)
    .execute(pool)
    .await?;
    get_last_read_at(pool, member_id).await
}

pub async fn unread_count(pool: &SqlitePool, member_id: MemberId) -> Result<i64, StoreError> {
    let last_read = get_last_read_at(pool, member_id).await?;
    let last_s = last_read.to_rfc3339();
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM maidan_mentions
         WHERE member_id = ? AND created_at > ?",
    )
    .bind(member_id.0)
    .bind(last_s)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn list_for_member(
    pool: &SqlitePool,
    member_id: MemberId,
    limit: i64,
) -> Result<MemberInbox, StoreError> {
    let last_read = get_last_read_at(pool, member_id).await?;
    let unread = unread_count(pool, member_id).await?;
    let rows = sqlx::query(
        "SELECT m.message_id, m.member_id, m.created_at,
                msg.body AS message_body, msg.thread_id, msg.author_id,
                t.channel_id, author.handle AS author_handle
         FROM maidan_mentions m
         JOIN maidan_messages msg ON msg.id = m.message_id
         JOIN maidan_threads t ON t.id = msg.thread_id
         JOIN maidan_members author ON author.id = msg.author_id
         WHERE m.member_id = ?
         ORDER BY m.created_at DESC
         LIMIT ?",
    )
    .bind(member_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let items = rows
        .iter()
        .map(|row| {
            let created_at: DateTime<Utc> = row.get("created_at");
            InboxItem {
                kind: InboxItemKind::Mention,
                message_id: MessageId(row.get("message_id")),
                member_id: MemberId(row.get("member_id")),
                created_at,
                unread: created_at > last_read,
                message_body: row.get("message_body"),
                thread_id: ThreadId(row.get("thread_id")),
                channel_id: ChannelId(row.get("channel_id")),
                author_id: MemberId(row.get("author_id")),
                author_handle: row.get("author_handle"),
            }
        })
        .collect();
    Ok(MemberInbox {
        items,
        unread_count: unread,
        last_read_at: last_read,
    })
}
