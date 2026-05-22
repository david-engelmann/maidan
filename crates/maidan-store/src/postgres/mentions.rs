use chrono::{DateTime, Utc};
use maidan_types::{MemberId, Mention, MessageId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn record(
    pool: &PgPool,
    message_id: MessageId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_mentions (message_id, member_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(message_id.0)
    .bind(member_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_for_member(
    pool: &PgPool,
    member_id: MemberId,
    limit: i64,
) -> Result<Vec<Mention>, StoreError> {
    let rows = sqlx::query(
        "SELECT message_id, member_id, created_at
         FROM maidan_mentions
         WHERE member_id = $1
         ORDER BY created_at DESC
         LIMIT $2",
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
