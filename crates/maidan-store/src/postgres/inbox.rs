use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, InboxItem, InboxItemKind, MemberId, MemberInbox, MessageId, ThreadId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

const EPOCH: &str = "1970-01-01T00:00:00Z";

pub async fn get_last_read_at(
    pool: &PgPool,
    member_id: MemberId,
) -> Result<DateTime<Utc>, StoreError> {
    let row: Option<(DateTime<Utc>,)> =
        sqlx::query_as("SELECT last_read_at FROM maidan_inbox_cursor WHERE member_id = $1")
            .bind(member_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0).unwrap_or_else(|| {
        DateTime::parse_from_rfc3339(EPOCH)
            .expect("epoch")
            .with_timezone(&Utc)
    }))
}

pub async fn advance_last_read_at(
    pool: &PgPool,
    member_id: MemberId,
    read_through: DateTime<Utc>,
) -> Result<DateTime<Utc>, StoreError> {
    let row: (DateTime<Utc>,) = sqlx::query_as(
        "INSERT INTO maidan_inbox_cursor (member_id, last_read_at, updated_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT (member_id)
         DO UPDATE SET
           last_read_at = GREATEST(maidan_inbox_cursor.last_read_at, EXCLUDED.last_read_at),
           updated_at = NOW()
         RETURNING last_read_at",
    )
    .bind(member_id.0)
    .bind(read_through)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn unread_count(pool: &PgPool, member_id: MemberId) -> Result<i64, StoreError> {
    let last_read = get_last_read_at(pool, member_id).await?;
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::bigint
         FROM maidan_mentions
         WHERE member_id = $1 AND created_at > $2",
    )
    .bind(member_id.0)
    .bind(last_read)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub async fn list_for_member(
    pool: &PgPool,
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
         WHERE m.member_id = $1
         ORDER BY m.created_at DESC
         LIMIT $2",
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
                message_id: MessageId(row.get::<Uuid, _>("message_id")),
                member_id: MemberId(row.get::<Uuid, _>("member_id")),
                created_at,
                unread: created_at > last_read,
                message_body: row.get("message_body"),
                thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
                channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
                author_id: MemberId(row.get::<Uuid, _>("author_id")),
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
