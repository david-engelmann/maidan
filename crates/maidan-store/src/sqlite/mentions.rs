use chrono::{DateTime, Utc};
use maidan_types::{MemberId, Mention, MessageId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn record(
    pool: &SqlitePool,
    message_id: MessageId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_mentions (message_id, member_id, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(message_id.0)
    .bind(member_id.0)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_for_member(
    pool: &SqlitePool,
    member_id: MemberId,
    limit: i64,
) -> Result<Vec<Mention>, StoreError> {
    let rows = sqlx::query(
        "SELECT message_id, member_id, created_at
         FROM maidan_mentions
         WHERE member_id = ?
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(member_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Mention {
            message_id: MessageId(row.get::<Uuid, _>("message_id")),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}
