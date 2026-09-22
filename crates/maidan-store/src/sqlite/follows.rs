use chrono::{DateTime, Utc};
use maidan_types::{ChannelFollow, ChannelId, MemberFollow, MemberId, ThreadFollow, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Follow a channel. Idempotent.
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

/// Unfollow a channel; `true` when a row was deleted.
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

/// The channels a member follows, newest first.
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

/// The members following a channel — the router's fan-out set.
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

/// Follow a thread. Idempotent.
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

/// Unfollow a thread; `true` when a row was deleted.
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

/// The threads a member follows, newest first.
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

/// The members following a thread — the router's fan-out set.
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

pub async fn follow_member(
    pool: &SqlitePool,
    follower_id: MemberId,
    followed_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_member_follows (follower_id, followed_id)
         VALUES (?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(follower_id.0)
    .bind(followed_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unfollow_member(
    pool: &SqlitePool,
    follower_id: MemberId,
    followed_id: MemberId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_member_follows WHERE follower_id = ? AND followed_id = ?")
            .bind(follower_id.0)
            .bind(followed_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_member_follows(
    pool: &SqlitePool,
    follower_id: MemberId,
) -> Result<Vec<MemberFollow>, StoreError> {
    let rows = sqlx::query(
        "SELECT follower_id, followed_id, created_at FROM maidan_member_follows
         WHERE follower_id = ? ORDER BY datetime(created_at) DESC, followed_id",
    )
    .bind(follower_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(MemberFollow {
                follower_id: MemberId(r.get::<Uuid, _>("follower_id")),
                followed_id: MemberId(r.get::<Uuid, _>("followed_id")),
                created_at: r.get::<DateTime<Utc>, _>("created_at"),
            })
        })
        .collect()
}

pub async fn member_followers(
    pool: &SqlitePool,
    followed_id: MemberId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query(
        "SELECT follower_id FROM maidan_member_follows WHERE followed_id = ? ORDER BY follower_id",
    )
    .bind(followed_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("follower_id")))
        .collect())
}

/// Mute a specific thread for a member. Idempotent.
pub async fn mute_thread(
    pool: &SqlitePool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_mutes (member_id, thread_id)
         VALUES (?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(thread_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unmute_thread(
    pool: &SqlitePool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_thread_mutes WHERE member_id = ? AND thread_id = ?")
        .bind(member_id.0)
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn is_thread_muted(
    pool: &SqlitePool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let row =
        sqlx::query("SELECT 1 FROM maidan_thread_mutes WHERE member_id = ? AND thread_id = ?")
            .bind(member_id.0)
            .bind(thread_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

pub async fn thread_muters(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_thread_mutes WHERE thread_id = ?")
        .bind(thread_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

/// Mute a whole channel for a member. Idempotent.
pub async fn mute_channel(
    pool: &SqlitePool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_channel_mutes (member_id, channel_id)
         VALUES (?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(channel_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unmute_channel(
    pool: &SqlitePool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_channel_mutes WHERE member_id = ? AND channel_id = ?")
            .bind(member_id.0)
            .bind(channel_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn is_channel_muted(
    pool: &SqlitePool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<bool, StoreError> {
    let row =
        sqlx::query("SELECT 1 FROM maidan_channel_mutes WHERE member_id = ? AND channel_id = ?")
            .bind(member_id.0)
            .bind(channel_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

pub async fn channel_muters(
    pool: &SqlitePool,
    channel_id: ChannelId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_channel_mutes WHERE channel_id = ?")
        .bind(channel_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}
