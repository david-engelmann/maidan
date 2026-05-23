use maidan_types::{Event, StoredEvent, WorkspaceId};
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;

pub async fn append(pool: &SqlitePool, event: &Event) -> Result<StoredEvent, StoreError> {
    let payload = serde_json::to_string(event)?;
    let row = sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id, kind, workspace_id, channel_id, thread_id, payload, occurred_at",
    )
    .bind(event.kind().as_str())
    .bind(event.workspace_id().map(|w| w.0))
    .bind(event.channel_id().map(|c| c.0))
    .bind(event.thread_id().map(|t| t.0))
    .bind(payload)
    .bind(event.occurred_at().to_rfc3339())
    .fetch_one(pool)
    .await?;
    row_to_stored(&row)
}

pub async fn list_after(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE workspace_id = ? AND id > ?
         ORDER BY id ASC
         LIMIT ?",
    )
    .bind(workspace_id.0)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_stored).collect()
}

fn row_to_stored(row: &sqlx::sqlite::SqliteRow) -> Result<StoredEvent, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = parse_kind(&kind_str)?;
    let payload: String = row.get("payload");
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
        payload: serde_json::from_str(&payload)?,
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
