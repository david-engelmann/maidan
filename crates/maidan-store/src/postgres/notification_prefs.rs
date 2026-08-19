use chrono::{DateTime, Utc};
use maidan_types::{EventKind, MemberId, NotificationPref};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a member's mute preference for an event kind (Cluster 241) — see the
/// SQLite twin.
pub async fn set(
    pool: &PgPool,
    member_id: MemberId,
    kind: EventKind,
    muted: bool,
) -> Result<NotificationPref, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_notification_prefs (member_id, kind, muted)
         VALUES ($1, $2, $3)
         ON CONFLICT (member_id, kind) DO UPDATE SET
             muted = excluded.muted,
             updated_at = now()
         RETURNING member_id, kind, muted, updated_at",
    )
    .bind(member_id.0)
    .bind(kind.as_str())
    .bind(muted)
    .fetch_one(pool)
    .await?;
    row_to_pref(&row)
}

pub async fn list(pool: &PgPool, member_id: MemberId) -> Result<Vec<NotificationPref>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, kind, muted, updated_at
         FROM maidan_notification_prefs
         WHERE member_id = $1
         ORDER BY kind ASC",
    )
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_pref).collect()
}

pub async fn is_muted(
    pool: &PgPool,
    member_id: MemberId,
    kind: EventKind,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT muted FROM maidan_notification_prefs WHERE member_id = $1 AND kind = $2",
    )
    .bind(member_id.0)
    .bind(kind.as_str())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get::<bool, _>("muted")).unwrap_or(false))
}

fn row_to_pref(row: &sqlx::postgres::PgRow) -> Result<NotificationPref, StoreError> {
    let kind_str: String = row.get("kind");
    Ok(NotificationPref {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        kind: EventKind::parse(&kind_str).ok_or_else(|| {
            StoreError::InvalidInput(format!("unknown notification kind: {kind_str}"))
        })?,
        muted: row.get::<bool, _>("muted"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
