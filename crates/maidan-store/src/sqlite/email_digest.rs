//! Email digest data model (Cluster 254, Program C — Arc I): a per-member
//! delivery-mode preference and a per-member digest watermark, plus the sweeper's
//! "due for digest" enumeration. Foundation only — no worker/routes wire it yet.

use chrono::{DateTime, Utc};
use maidan_types::{DigestDue, EmailDeliveryMode, MemberId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// Set a member's email delivery mode (Cluster 254). Upsert — one row per member.
pub async fn set_delivery_mode(
    pool: &SqlitePool,
    member_id: MemberId,
    mode: EmailDeliveryMode,
) -> Result<(), crate::error::StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_member_delivery_prefs (member_id, mode, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT (member_id) DO UPDATE SET mode = excluded.mode, updated_at = excluded.updated_at",
    )
    .bind(member_id.0)
    .bind(mode.as_str())
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// A member's delivery mode, defaulting to `Immediate` when unset (Cluster 254).
pub async fn get_delivery_mode(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<EmailDeliveryMode, crate::error::StoreError> {
    let row = sqlx::query("SELECT mode FROM maidan_member_delivery_prefs WHERE member_id = ?")
        .bind(member_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row
        .map(|r| r.get::<String, _>("mode"))
        .and_then(|s| EmailDeliveryMode::parse(&s))
        .unwrap_or_default())
}

/// Advance a member's digest watermark to `now` (Cluster 254) — called after a
/// digest is emailed, so the next run only counts notifications created since.
pub async fn set_last_digest_at(
    pool: &SqlitePool,
    member_id: MemberId,
    now: DateTime<Utc>,
) -> Result<(), crate::error::StoreError> {
    let now = now.to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_member_digest_state (member_id, last_digest_at)
         VALUES (?, ?)
         ON CONFLICT (member_id) DO UPDATE SET last_digest_at = excluded.last_digest_at",
    )
    .bind(member_id.0)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Members due for an email digest (Cluster 254): digest-mode members with an
/// address who have unread notifications created since their last digest, most-
/// unread irrelevant — ordered by member id, capped at `limit`. `datetime(...)`
/// wraps both sides so the `datetime('now')`-formatted `created_at` and the
/// rfc3339 `last_digest_at` compare correctly.
pub async fn members_due_for_digest(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<DigestDue>, crate::error::StoreError> {
    let rows = sqlx::query(
        "SELECT n.member_id AS member_id, e.email AS email, COUNT(*) AS unread
         FROM maidan_notifications n
         JOIN maidan_member_emails e ON e.member_id = n.member_id
         JOIN maidan_member_delivery_prefs p
             ON p.member_id = n.member_id AND p.mode = 'digest'
         LEFT JOIN maidan_member_digest_state d ON d.member_id = n.member_id
         WHERE n.read_at IS NULL
           AND datetime(n.created_at) > datetime(COALESCE(d.last_digest_at, '1970-01-01 00:00:00'))
         GROUP BY n.member_id, e.email
         HAVING COUNT(*) > 0
         ORDER BY n.member_id
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DigestDue {
            member_id: MemberId(r.get::<Uuid, _>("member_id")),
            email: r.get::<String, _>("email"),
            unread_count: r.get::<i64, _>("unread"),
        })
        .collect())
}
