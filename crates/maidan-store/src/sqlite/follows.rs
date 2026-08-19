use chrono::{DateTime, Utc};
use maidan_types::{ChannelFollow, ChannelId, MemberId, ThreadFollow, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Follow a channel (Cluster 244). Idempotent.
pub async fn follow_channel(
    pool: &SqlitePool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_channel_follows (member_id, channel_id, created_at)
         VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(channel_id.0)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Unfollow a channel; `true` when a row was deleted (Cluster 244).
pub async fn unfollow_channel(
    pool: &SqlitePool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_channel_follows WHERE member_id = ? AND channel_id = ?")
            .bind(member_id.0)
            .bind(channel_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

/// The channels a member follows, newest first (Cluster 244).
pub async fn list_channel_follows(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Vec<ChannelFollow>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, channel_id, created_at FROM maidan_channel_follows
         WHERE member_id = ? ORDER BY created_at DESC",
    )
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| ChannelFollow {
            member_id: MemberId(r.get::<Uuid, _>("member_id")),
            channel_id: ChannelId(r.get::<Uuid, _>("channel_id")),
            created_at: r.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}

/// The members following a channel — the router's fan-out set (Cluster 244).
pub async fn channel_followers(
    pool: &SqlitePool,
    channel_id: ChannelId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_channel_follows WHERE channel_id = ?")
        .bind(channel_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

/// Follow a thread (Cluster 244). Idempotent.
pub async fn follow_thread(
    pool: &SqlitePool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_follows (member_id, thread_id, created_at)
         VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(thread_id.0)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Unfollow a thread; `true` when a row was deleted (Cluster 244).
pub async fn unfollow_thread(
    pool: &SqlitePool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_thread_follows WHERE member_id = ? AND thread_id = ?")
            .bind(member_id.0)
            .bind(thread_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

/// The threads a member follows, newest first (Cluster 244).
pub async fn list_thread_follows(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Vec<ThreadFollow>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, thread_id, created_at FROM maidan_thread_follows
         WHERE member_id = ? ORDER BY created_at DESC",
    )
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| ThreadFollow {
            member_id: MemberId(r.get::<Uuid, _>("member_id")),
            thread_id: ThreadId(r.get::<Uuid, _>("thread_id")),
            created_at: r.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}

/// The members following a thread — the router's fan-out set (Cluster 244).
pub async fn thread_followers(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_thread_follows WHERE thread_id = ?")
        .bind(thread_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}
