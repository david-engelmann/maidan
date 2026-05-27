use maidan_types::{Event, StoredEvent, WorkspaceId};
use sqlx::{PgPool, Row};

use crate::error::StoreError;

pub async fn append(pool: &PgPool, event: &Event) -> Result<StoredEvent, StoreError> {
    let payload = serde_json::to_value(event)?;
    let row = sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, kind, workspace_id, channel_id, thread_id, payload, occurred_at",
    )
    .bind(event.kind().as_str())
    .bind(event.workspace_id().map(|w| w.0))
    .bind(event.channel_id().map(|c| c.0))
    .bind(event.thread_id().map(|t| t.0))
    .bind(payload)
    .bind(event.occurred_at())
    .fetch_one(pool)
    .await?;
    row_to_stored(&row)
}

pub async fn get_by_id(pool: &PgPool, log_id: i64) -> Result<StoredEvent, StoreError> {
    let row = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE id = $1",
    )
    .bind(log_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    row_to_stored(&row)
}

pub async fn list_after(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE workspace_id = $1 AND id > $2
         ORDER BY id ASC
         LIMIT $3",
    )
    .bind(workspace_id.0)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_stored).collect()
}

fn row_to_stored(row: &sqlx::postgres::PgRow) -> Result<StoredEvent, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = parse_kind(&kind_str)?;
    Ok(StoredEvent {
        id: row.get("id"),
        kind,
        workspace_id: row
            .get::<Option<uuid::Uuid>, _>("workspace_id")
            .map(maidan_types::WorkspaceId),
        channel_id: row
            .get::<Option<uuid::Uuid>, _>("channel_id")
            .map(maidan_types::ChannelId),
        thread_id: row
            .get::<Option<uuid::Uuid>, _>("thread_id")
            .map(maidan_types::ThreadId),
        payload: row.get("payload"),
        occurred_at: row.get("occurred_at"),
    })
}

fn parse_kind(s: &str) -> Result<maidan_types::EventKind, StoreError> {
    use maidan_types::EventKind;
    match s {
        "workspace_created" => Ok(EventKind::WorkspaceCreated),
        "member_joined" => Ok(EventKind::MemberJoined),
        "channel_created" => Ok(EventKind::ChannelCreated),
        "thread_created" => Ok(EventKind::ThreadCreated),
        "thread_state_changed" => Ok(EventKind::ThreadStateChanged),
        "message_posted" => Ok(EventKind::MessagePosted),
        "message_tombstoned" => Ok(EventKind::MessageTombstoned),
        "mention_recorded" => Ok(EventKind::MentionRecorded),
        "vote_cast" => Ok(EventKind::VoteCast),
        "reference_added" => Ok(EventKind::ReferenceAdded),
        "artifact_upserted" => Ok(EventKind::ArtifactUpserted),
        other => Err(StoreError::InvalidInput(format!(
            "unknown event kind: {other}"
        ))),
    }
}
