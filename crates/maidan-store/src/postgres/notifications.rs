use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, EventKind, MemberId, MessageId, NewNotification, Notification, NotificationId,
    ThreadId, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Insert one per-recipient notification (Cluster 237) — see the SQLite twin.
pub async fn create(pool: &PgPool, new: NewNotification) -> Result<Notification, StoreError> {
    let id = NotificationId::new();
    let row = sqlx::query(
        "INSERT INTO maidan_notifications
            (id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
             message_id, actor_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
    .fetch_one(pool)
    .await?;
    row_to_notification(&row)
}

/// Insert unless one already exists for `(member_id, source_log_id)` (Cluster 238) —
/// see the SQLite twin. `None` = a row already existed (deduped).
pub async fn create_if_absent(
    pool: &PgPool,
    new: NewNotification,
) -> Result<Option<Notification>, StoreError> {
    let id = NotificationId::new();
    let row = sqlx::query(
        "INSERT INTO maidan_notifications
            (id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
             message_id, actor_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_notification).transpose()
}

/// Insert many notifications in one round trip (Cluster 349) — the batch form of
/// [`create_if_absent`] for the `MessagePosted` fan-out. Each row is
/// `ON CONFLICT (member_id, source_log_id) DO NOTHING`; returns the actually-
/// inserted rows (deduped rows omitted). The caller passes a set of distinct
/// recipients (so no intra-batch key collision). Inserts via `UNNEST` arrays, so
/// row count never affects the bound-parameter count (9 array params).
pub async fn create_batch(
    pool: &PgPool,
    rows: &[NewNotification],
) -> Result<Vec<Notification>, StoreError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = rows.iter().map(|_| NotificationId::new().0).collect();
    let workspace_ids: Vec<Uuid> = rows.iter().map(|r| r.workspace_id.0).collect();
    let member_ids: Vec<Uuid> = rows.iter().map(|r| r.member_id.0).collect();
    let kinds: Vec<String> = rows.iter().map(|r| r.kind.as_str().to_string()).collect();
    let source_log_ids: Vec<i64> = rows.iter().map(|r| r.source_log_id).collect();
    let channel_ids: Vec<Option<Uuid>> = rows.iter().map(|r| r.channel_id.map(|c| c.0)).collect();
    let thread_ids: Vec<Option<Uuid>> = rows.iter().map(|r| r.thread_id.map(|t| t.0)).collect();
    let message_ids: Vec<Option<Uuid>> = rows.iter().map(|r| r.message_id.map(|m| m.0)).collect();
    let actor_ids: Vec<Option<Uuid>> = rows.iter().map(|r| r.actor_id.map(|a| a.0)).collect();
    let inserted = sqlx::query(
        "INSERT INTO maidan_notifications
            (id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
             message_id, actor_id)
         SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[], $5::bigint[],
                              $6::uuid[], $7::uuid[], $8::uuid[], $9::uuid[])
         ON CONFLICT (member_id, source_log_id) DO NOTHING
         RETURNING id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                   message_id, actor_id, created_at, read_at",
    )
    .bind(&ids)
    .bind(&workspace_ids)
    .bind(&member_ids)
    .bind(&kinds)
    .bind(&source_log_ids)
    .bind(&channel_ids)
    .bind(&thread_ids)
    .bind(&message_ids)
    .bind(&actor_ids)
    .fetch_all(pool)
    .await?;
    inserted.iter().map(row_to_notification).collect()
}

pub async fn list_for_member(
    pool: &PgPool,
    member_id: MemberId,
    unread_only: bool,
    limit: i64,
) -> Result<Vec<Notification>, StoreError> {
    let sql = if unread_only {
        "SELECT id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                message_id, actor_id, created_at, read_at
         FROM maidan_notifications
         WHERE member_id = $1 AND read_at IS NULL
         ORDER BY created_at DESC LIMIT $2"
    } else {
        "SELECT id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                message_id, actor_id, created_at, read_at
         FROM maidan_notifications
         WHERE member_id = $1
         ORDER BY created_at DESC LIMIT $2"
    };
    let rows = sqlx::query(sql)
        .bind(member_id.0)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter().map(row_to_notification).collect()
}

/// Idempotent + recipient-scoped (Cluster 237/239) — a re-mark preserves the original
/// `read_at`; returns whether a `(member_id, id)` row exists (so a caller can't mark
/// another member's notification).
pub async fn mark_read(
    pool: &PgPool,
    member_id: MemberId,
    id: NotificationId,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_notifications SET read_at = COALESCE(read_at, now())
         WHERE id = $1 AND member_id = $2",
    )
    .bind(id.0)
    .bind(member_id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn mark_all_read(pool: &PgPool, member_id: MemberId) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_notifications SET read_at = now() WHERE member_id = $1 AND read_at IS NULL",
    )
    .bind(member_id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn unread_count(pool: &PgPool, member_id: MemberId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM maidan_notifications WHERE member_id = $1 AND read_at IS NULL",
    )
    .bind(member_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}

fn row_to_notification(row: &sqlx::postgres::PgRow) -> Result<Notification, StoreError> {
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
