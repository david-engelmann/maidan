use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, DlqEntry, DlqEntryId, MemberId, NewDlqEntry, ThreadId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "id, workspace_id, channel_id, thread_id, member_id, reason, \
     used_tokens, used_usd_micros, used_turns, failed_at";

/// Record a dead-lettered agent run (Cluster 358, T1/T5). `id`/`failed_at` are
/// assigned here; `failed_at` is bound rfc3339 (not the `datetime('now')` default)
/// so it reads back as `DateTime<Utc>` cleanly.
pub async fn record(pool: &SqlitePool, new: &NewDlqEntry) -> Result<DlqEntry, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_agent_work_dlq
             (id, workspace_id, channel_id, thread_id, member_id, reason,
              used_tokens, used_usd_micros, used_turns, failed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, channel_id, thread_id, member_id, reason, \
             used_tokens, used_usd_micros, used_turns, failed_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(new.thread_id.0)
    .bind(new.member_id.0)
    .bind(&new.reason)
    .bind(new.used_tokens)
    .bind(new.used_usd_micros)
    .bind(new.used_turns)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_dlq(&row))
}

/// A channel's dead-lettered runs, newest first (Cluster 358).
pub async fn list_for_channel(
    pool: &SqlitePool,
    channel_id: ChannelId,
    limit: i64,
) -> Result<Vec<DlqEntry>, StoreError> {
    let sql = format!(
        "SELECT {COLS} FROM maidan_agent_work_dlq
         WHERE channel_id = ? ORDER BY failed_at DESC, id DESC LIMIT ?"
    );
    let rows = sqlx::query(&sql)
        .bind(channel_id.0)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(row_to_dlq).collect())
}

fn row_to_dlq(row: &sqlx::sqlite::SqliteRow) -> DlqEntry {
    DlqEntry {
        id: DlqEntryId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        reason: row.get::<String, _>("reason"),
        used_tokens: row.get::<i64, _>("used_tokens"),
        used_usd_micros: row.get::<i64, _>("used_usd_micros"),
        used_turns: row.get::<i64, _>("used_turns"),
        failed_at: row.get::<DateTime<Utc>, _>("failed_at"),
    }
}
