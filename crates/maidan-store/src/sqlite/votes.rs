use chrono::{DateTime, Utc};
use maidan_types::{MemberId, MessageId, NewVote, Vote};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn cast(pool: &SqlitePool, new: NewVote) -> Result<(), StoreError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_votes (message_id, member_id, kind, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .bind(&new.kind)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool, message_id: MessageId) -> Result<Vec<Vote>, StoreError> {
    let rows = sqlx::query(
        "SELECT message_id, member_id, kind, created_at
         FROM maidan_votes
         WHERE message_id = ?
         ORDER BY created_at ASC",
    )
    .bind(message_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Vote {
            message_id: MessageId(row.get::<Uuid, _>("message_id")),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            kind: row.get("kind"),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}
