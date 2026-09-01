use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, EventKind, MemberId, MessageId, NewNotification, Notification, NotificationId,
    ThreadId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Insert one per-recipient notification (Cluster 237).
pub async fn create(pool: &SqlitePool, new: NewNotification) -> Result<Notification, StoreError> {
    let id = NotificationId::new();
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_notifications
            (id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
             message_id, actor_id, created_at, read_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
         RETURNING id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                   message_id, actor_id, created_at, read_at",
    )
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(new.kind.as_str())
    .bind(new.source_log_id)
    .bind(new.channel_id.map(|c| c.0))
    .bind(new.thread_id.map(|t| t.0))
    .bind(new.message_id.map(|m| m.0))
    .bind(new.actor_id.map(|a| a.0))
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_notification(&row)
}

/// Insert unless one already exists for `(member_id, source_log_id)` (Cluster 238).
/// `None` = a row already existed (deduped — a replay or a second replica).
pub async fn create_if_absent(
    pool: &SqlitePool,
    new: NewNotification,
) -> Result<Option<Notification>, StoreError> {
    let id = NotificationId::new();
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_notifications
            (id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
             message_id, actor_id, created_at, read_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
         ON CONFLICT (member_id, source_log_id) DO NOTHING
         RETURNING id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                   message_id, actor_id, created_at, read_at",
    )
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(new.kind.as_str())
    .bind(new.source_log_id)
    .bind(new.channel_id.map(|c| c.0))
    .bind(new.thread_id.map(|t| t.0))
    .bind(new.message_id.map(|m| m.0))
    .bind(new.actor_id.map(|a| a.0))
    .bind(&now)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_notification).transpose()
}

/// Insert many notifications in one round trip (Cluster 349) — the batch form of
/// [`create_if_absent`] for the `MessagePosted` fan-out. Each row is
/// `ON CONFLICT (member_id, source_log_id) DO NOTHING`; returns the actually-
/// inserted rows (deduped rows omitted). The caller passes a set of distinct
/// recipients (so no intra-batch key collision). Chunked under SQLite's
/// 999-parameter limit (10 bound params per row → 90 rows/chunk).
pub async fn create_batch(
    pool: &SqlitePool,
    rows: &[NewNotification],
) -> Result<Vec<Notification>, StoreError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    const COLS: &str = "id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id, \
                        message_id, actor_id, created_at, read_at";
    const CHUNK: usize = 90;
    let now = Utc::now().to_rfc3339();
    let mut out = Vec::with_capacity(rows.len());
    for chunk in rows.chunks(CHUNK) {
        let values = vec!["(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)"; chunk.len()].join(", ");
        let sql = format!(
            "INSERT INTO maidan_notifications ({COLS})
             VALUES {values}
             ON CONFLICT (member_id, source_log_id) DO NOTHING
             RETURNING {COLS}"
        );
        let mut q = sqlx::query(&sql);
        for new in chunk {
            q = q
                .bind(NotificationId::new().0)
                .bind(new.workspace_id.0)
                .bind(new.member_id.0)
                .bind(new.kind.as_str())
                .bind(new.source_log_id)
                .bind(new.channel_id.map(|c| c.0))
                .bind(new.thread_id.map(|t| t.0))
                .bind(new.message_id.map(|m| m.0))
                .bind(new.actor_id.map(|a| a.0))
                .bind(now.clone());
        }
        let inserted = q.fetch_all(pool).await?;
        for r in &inserted {
            out.push(row_to_notification(r)?);
        }
    }
    Ok(out)
}

/// A member's notifications, newest first, optionally unread-only (Cluster 237).
pub async fn list_for_member(
    pool: &SqlitePool,
    member_id: MemberId,
    unread_only: bool,
    limit: i64,
) -> Result<Vec<Notification>, StoreError> {
    let sql = if unread_only {
        "SELECT id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                message_id, actor_id, created_at, read_at
         FROM maidan_notifications
         WHERE member_id = ? AND read_at IS NULL
         ORDER BY created_at DESC LIMIT ?"
    } else {
        "SELECT id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                message_id, actor_id, created_at, read_at
         FROM maidan_notifications
         WHERE member_id = ?
         ORDER BY created_at DESC LIMIT ?"
    };
    let rows = sqlx::query(sql)
        .bind(member_id.0)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter().map(row_to_notification).collect()
}

/// Mark one notification read, scoped to its recipient (Cluster 237/239). Idempotent
/// — a re-mark preserves the original `read_at`. Returns whether a `(member_id, id)`
/// row exists (so a caller can't mark another member's notification).
pub async fn mark_read(
    pool: &SqlitePool,
    member_id: MemberId,
    id: NotificationId,
) -> Result<bool, StoreError> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "UPDATE maidan_notifications SET read_at = COALESCE(read_at, ?)
         WHERE id = ? AND member_id = ?",
    )
    .bind(&now)
    .bind(id.0)
    .bind(member_id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Mark all of a member's unread notifications read (Cluster 237). Returns the
/// number cleared.
pub async fn mark_all_read(pool: &SqlitePool, member_id: MemberId) -> Result<u64, StoreError> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "UPDATE maidan_notifications SET read_at = ? WHERE member_id = ? AND read_at IS NULL",
    )
    .bind(&now)
    .bind(member_id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// The unread-notification badge count for a member (Cluster 237).
pub async fn unread_count(pool: &SqlitePool, member_id: MemberId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM maidan_notifications WHERE member_id = ? AND read_at IS NULL",
    )
    .bind(member_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}

fn row_to_notification(row: &sqlx::sqlite::SqliteRow) -> Result<Notification, StoreError> {
    let kind_str: String = row.get("kind");
    Ok(Notification {
        id: NotificationId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        kind: EventKind::parse(&kind_str).ok_or_else(|| {
            StoreError::InvalidInput(format!("unknown notification kind: {kind_str}"))
        })?,
        source_log_id: row.get::<i64, _>("source_log_id"),
        channel_id: row.get::<Option<Uuid>, _>("channel_id").map(ChannelId),
        thread_id: row.get::<Option<Uuid>, _>("thread_id").map(ThreadId),
        message_id: row.get::<Option<Uuid>, _>("message_id").map(MessageId),
        actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        read_at: row.get::<Option<DateTime<Utc>>, _>("read_at"),
    })
}
