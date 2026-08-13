use chrono::{DateTime, Utc};
use maidan_types::{Event, MemberId, MessageId, NewVote, StoredEvent, Vote};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

pub async fn cast(pool: &PgPool, new: NewVote) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_votes (message_id, member_id, kind)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .bind(&new.kind)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cast a vote and append its `VoteCast` event in one transaction (Cluster 206).
pub async fn cast_with_event(pool: &PgPool, new: NewVote) -> Result<StoredEvent, StoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_votes (message_id, member_id, kind)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .bind(&new.kind)
    .execute(&mut *tx)
    .await?;
    let (workspace_id, _channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, new.message_id).await?;
    let event = Event::VoteCast {
        occurred_at: Utc::now(),
        workspace_id,
        thread_id,
        message_id: new.message_id,
        member_id: new.member_id,
        vote_kind: new.kind.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(stored)
}

pub async fn list(pool: &PgPool, message_id: MessageId) -> Result<Vec<Vote>, StoreError> {
    let rows = sqlx::query(
        "SELECT message_id, member_id, kind, created_at
         FROM maidan_votes
         WHERE message_id = $1
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
