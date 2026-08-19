use chrono::{DateTime, Utc};
use maidan_types::{EventKind, MemberId, NotificationPref};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a member's mute preference for an event kind (Cluster 241). A re-set
/// overwrites.
pub async fn set(
    pool: &SqlitePool,
    member_id: MemberId,
    kind: EventKind,
    muted: bool,
) -> Result<NotificationPref, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_notification_prefs (member_id, kind, muted, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (member_id, kind) DO UPDATE SET
             muted = excluded.muted,
             updated_at = excluded.updated_at
         RETURNING member_id, kind, muted, updated_at",
    )
    .bind(member_id.0)
    .bind(kind.as_str())
    .bind(muted)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_pref(&row)
}

/// A member's notification preferences, ordered by kind (Cluster 241).
pub async fn list(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Vec<NotificationPref>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, kind, muted, updated_at
         FROM maidan_notification_prefs
         WHERE member_id = ?
         ORDER BY kind ASC",
    )
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_pref).collect()
}

/// Whether a member has muted an event kind (Cluster 241). Absent row = not muted.
pub async fn is_muted(
    pool: &SqlitePool,
    member_id: MemberId,
    kind: EventKind,
) -> Result<bool, StoreError> {
    let row =
        sqlx::query("SELECT muted FROM maidan_notification_prefs WHERE member_id = ? AND kind = ?")
            .bind(member_id.0)
            .bind(kind.as_str())
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.get::<bool, _>("muted")).unwrap_or(false))
}

fn row_to_pref(row: &sqlx::sqlite::SqliteRow) -> Result<NotificationPref, StoreError> {
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
