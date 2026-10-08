use chrono::{DateTime, Utc};
use maidan_types::{Event, MemberId, MessageId, NewVote, StoredEvent, Vote, VoteKind};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

/// The verdict a member's new verdict replaces: `approve` and
/// `request_changes` exclude each other, `ack` stands beside either.
fn opposing(kind: VoteKind) -> Option<VoteKind> {
    match kind {
        VoteKind::Approve => Some(VoteKind::RequestChanges),
        VoteKind::RequestChanges => Some(VoteKind::Approve),
        VoteKind::Ack => None,
    }
}

/// Upsert the vote and drop the member's opposing verdict, returning the
/// verdict it replaced.
async fn cast_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    new: &NewVote,
) -> Result<Option<VoteKind>, StoreError> {
    let mut replaced = None;
    if let Some(other) = opposing(new.kind) {
        let removed = sqlx::query(
            "DELETE FROM maidan_votes WHERE message_id = ? AND member_id = ? AND kind = ?",
        )
        .bind(new.message_id.0)
        .bind(new.member_id.0)
        .bind(other.as_str())
        .execute(&mut **tx)
        .await?;
        if removed.rows_affected() > 0 {
            replaced = Some(other);
        }
    }
    sqlx::query(
        "INSERT INTO maidan_votes (message_id, member_id, kind, created_at, confidence)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (message_id, member_id, kind)
         DO UPDATE SET confidence = excluded.confidence",
    )
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .bind(new.kind.as_str())
    .bind(Utc::now())
    .bind(new.confidence)
    .execute(&mut **tx)
    .await?;
    Ok(replaced)
}

pub async fn cast(pool: &SqlitePool, new: NewVote) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    cast_in_tx(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(())
}

/// Cast a vote and append its events in one transaction: a `VoteRetracted`
/// for the verdict it replaced, if any, then its `VoteCast`.
pub async fn cast_with_event(
    pool: &SqlitePool,
    new: NewVote,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut tx = pool.begin().await?;
    let replaced = cast_in_tx(&mut tx, &new).await?;
    let (workspace_id, _channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, new.message_id).await?;
    let mut stored = Vec::with_capacity(2);
    if let Some(old) = replaced {
        let event = Event::VoteRetracted {
            occurred_at: Utc::now(),
            workspace_id,
            thread_id,
            message_id: new.message_id,
            member_id: new.member_id,
            vote_kind: old.as_str().to_string(),
        };
        stored.push(events::append_in_tx(&mut tx, &event).await?);
    }
    let event = Event::VoteCast {
        occurred_at: Utc::now(),
        workspace_id,
        thread_id,
        message_id: new.message_id,
        member_id: new.member_id,
        vote_kind: new.kind.as_str().to_string(),
    };
    stored.push(events::append_in_tx(&mut tx, &event).await?);
    tx.commit().await?;
    Ok(stored)
}

/// Take back a member's vote of one kind; append its `VoteRetracted` event in
/// the same transaction when a row was removed. Returns `(removed, event)`.
pub async fn retract_with_event(
    pool: &SqlitePool,
    message_id: MessageId,
    member_id: MemberId,
    kind: VoteKind,
) -> Result<(bool, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let result =
        sqlx::query("DELETE FROM maidan_votes WHERE message_id = ? AND member_id = ? AND kind = ?")
            .bind(message_id.0)
            .bind(member_id.0)
            .bind(kind.as_str())
            .execute(&mut *tx)
            .await?;
    if result.rows_affected() == 0 {
        tx.commit().await?;
        return Ok((false, None));
    }
    let (workspace_id, _channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, message_id).await?;
    let event = Event::VoteRetracted {
        occurred_at: Utc::now(),
        workspace_id,
        thread_id,
        message_id,
        member_id,
        vote_kind: kind.as_str().to_string(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((true, Some(stored)))
}

pub async fn list(pool: &SqlitePool, message_id: MessageId) -> Result<Vec<Vote>, StoreError> {
    let rows = sqlx::query(
        "SELECT message_id, member_id, kind, confidence, created_at
         FROM maidan_votes
         WHERE message_id = ?
         ORDER BY created_at ASC",
    )
    .bind(message_id.0)
    .fetch_all(pool)
    .await?;
    let mut votes = Vec::with_capacity(rows.len());
    for row in &rows {
        let kind: String = row.get("kind");
        votes.push(Vote {
            message_id: MessageId(row.get::<Uuid, _>("message_id")),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            kind: VoteKind::parse(&kind)
                .ok_or_else(|| StoreError::InvalidInput(format!("unknown vote kind: {kind}")))?,
            confidence: row.get::<Option<f64>, _>("confidence"),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
        });
    }
    Ok(votes)
}
