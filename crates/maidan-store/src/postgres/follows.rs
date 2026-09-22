use chrono::{DateTime, Utc};
use maidan_types::{ChannelFollow, ChannelId, MemberFollow, MemberId, ThreadFollow, ThreadId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Follow a channel. Idempotent.
pub async fn follow_channel(
    pool: &PgPool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_channel_follows (member_id, channel_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(channel_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unfollow_channel(
    pool: &PgPool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_channel_follows WHERE member_id = $1 AND channel_id = $2")
            .bind(member_id.0)
            .bind(channel_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_channel_follows(
    pool: &PgPool,
    member_id: MemberId,
) -> Result<Vec<ChannelFollow>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, channel_id, created_at FROM maidan_channel_follows
         WHERE member_id = $1 ORDER BY created_at DESC",
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

pub async fn channel_followers(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_channel_follows WHERE channel_id = $1")
        .bind(channel_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

pub async fn follow_thread(
    pool: &PgPool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_follows (member_id, thread_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(thread_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unfollow_thread(
    pool: &PgPool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_thread_follows WHERE member_id = $1 AND thread_id = $2")
            .bind(member_id.0)
            .bind(thread_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_thread_follows(
    pool: &PgPool,
    member_id: MemberId,
) -> Result<Vec<ThreadFollow>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, thread_id, created_at FROM maidan_thread_follows
         WHERE member_id = $1 ORDER BY created_at DESC",
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

pub async fn thread_followers(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_thread_follows WHERE thread_id = $1")
        .bind(thread_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

/// Follow another member's occupancy. Idempotent. Same-workspace and self-follow
/// policy are enforced at the server boundary; the schema rejects self-edges as
/// a final invariant.
pub async fn follow_member(
    pool: &PgPool,
    follower_id: MemberId,
    followed_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_member_follows (follower_id, followed_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(follower_id.0)
    .bind(followed_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unfollow_member(
    pool: &PgPool,
    follower_id: MemberId,
    followed_id: MemberId,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_member_follows WHERE follower_id = $1 AND followed_id = $2",
    )
    .bind(follower_id.0)
    .bind(followed_id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_member_follows(
    pool: &PgPool,
    follower_id: MemberId,
) -> Result<Vec<MemberFollow>, StoreError> {
    let rows = sqlx::query(
        "SELECT follower_id, followed_id, created_at FROM maidan_member_follows
         WHERE follower_id = $1 ORDER BY created_at DESC, followed_id",
    )
    .bind(follower_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| MemberFollow {
            follower_id: MemberId(r.get::<Uuid, _>("follower_id")),
            followed_id: MemberId(r.get::<Uuid, _>("followed_id")),
            created_at: r.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}

pub async fn member_followers(
    pool: &PgPool,
    followed_id: MemberId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query(
        "SELECT follower_id FROM maidan_member_follows WHERE followed_id = $1 ORDER BY follower_id",
    )
    .bind(followed_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("follower_id")))
        .collect())
}

/// Mute a specific thread for a member. Idempotent — the notification router
/// suppresses notifications about a muted thread.
pub async fn mute_thread(
    pool: &PgPool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_mutes (member_id, thread_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(thread_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unmute_thread(
    pool: &PgPool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_thread_mutes WHERE member_id = $1 AND thread_id = $2")
            .bind(member_id.0)
            .bind(thread_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn is_thread_muted(
    pool: &PgPool,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let row =
        sqlx::query("SELECT 1 FROM maidan_thread_mutes WHERE member_id = $1 AND thread_id = $2")
            .bind(member_id.0)
            .bind(thread_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

pub async fn thread_muters(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_thread_mutes WHERE thread_id = $1")
        .bind(thread_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}

/// Mute a whole channel for a member. Idempotent — the notification router
/// suppresses the channel's firehose, but a mention breaks through (357.2).
pub async fn mute_channel(
    pool: &PgPool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_channel_mutes (member_id, channel_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(channel_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unmute_channel(
    pool: &PgPool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_channel_mutes WHERE member_id = $1 AND channel_id = $2")
            .bind(member_id.0)
            .bind(channel_id.0)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn is_channel_muted(
    pool: &PgPool,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<bool, StoreError> {
    let row =
        sqlx::query("SELECT 1 FROM maidan_channel_mutes WHERE member_id = $1 AND channel_id = $2")
            .bind(member_id.0)
            .bind(channel_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

pub async fn channel_muters(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<Vec<MemberId>, StoreError> {
    let rows = sqlx::query("SELECT member_id FROM maidan_channel_mutes WHERE channel_id = $1")
        .bind(channel_id.0)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| MemberId(r.get::<Uuid, _>("member_id")))
        .collect())
}
