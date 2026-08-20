//! Email digest data model (Cluster 254, Program C — Arc I) — the Postgres twin
//! of the SQLite module. Foundation only — no worker/routes wire it yet.

use chrono::{DateTime, Utc};
use maidan_types::{DigestDue, EmailDeliveryMode, MemberId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Set a member's email delivery mode (Cluster 254). Upsert — one row per member.
pub async fn set_delivery_mode(
    pool: &PgPool,
    member_id: MemberId,
    mode: EmailDeliveryMode,
) -> Result<(), crate::error::StoreError> {
    sqlx::query(
        "INSERT INTO maidan_member_delivery_prefs (member_id, mode, updated_at)
         VALUES ($1, $2, now())
         ON CONFLICT (member_id) DO UPDATE SET mode = excluded.mode, updated_at = now()",
    )
    .bind(member_id.0)
    .bind(mode.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// A member's delivery mode, defaulting to `Immediate` when unset (Cluster 254).
pub async fn get_delivery_mode(
    pool: &PgPool,
    member_id: MemberId,
) -> Result<EmailDeliveryMode, crate::error::StoreError> {
    let row = sqlx::query("SELECT mode FROM maidan_member_delivery_prefs WHERE member_id = $1")
        .bind(member_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row
        .map(|r| r.get::<String, _>("mode"))
        .and_then(|s| EmailDeliveryMode::parse(&s))
        .unwrap_or_default())
}

/// Advance a member's digest watermark to `now` (Cluster 254).
pub async fn set_last_digest_at(
    pool: &PgPool,
    member_id: MemberId,
    now: DateTime<Utc>,
) -> Result<(), crate::error::StoreError> {
    sqlx::query(
        "INSERT INTO maidan_member_digest_state (member_id, last_digest_at)
         VALUES ($1, $2)
         ON CONFLICT (member_id) DO UPDATE SET last_digest_at = excluded.last_digest_at",
    )
    .bind(member_id.0)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Members due for an email digest (Cluster 254) — see the SQLite twin. Native
/// `timestamptz` comparison; `'epoch'` is the never-digested floor.
pub async fn members_due_for_digest(
    pool: &PgPool,
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
           AND n.created_at > COALESCE(d.last_digest_at, 'epoch'::timestamptz)
         GROUP BY n.member_id, e.email
         HAVING COUNT(*) > 0
         ORDER BY n.member_id
         LIMIT $1",
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
