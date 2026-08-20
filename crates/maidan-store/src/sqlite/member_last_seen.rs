use chrono::{DateTime, Utc};
use maidan_types::MemberId;
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;

/// Record that a member was just seen (Cluster 252). Upsert to `now()`.
pub async fn touch(pool: &SqlitePool, member_id: MemberId) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_member_last_seen (member_id, last_seen_at)
         VALUES (?, ?)
         ON CONFLICT (member_id) DO UPDATE SET last_seen_at = excluded.last_seen_at",
    )
    .bind(member_id.0)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// A member's last-seen instant, or `None` if never seen (Cluster 252).
pub async fn get(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Option<DateTime<Utc>>, StoreError> {
    let row = sqlx::query("SELECT last_seen_at FROM maidan_member_last_seen WHERE member_id = ?")
        .bind(member_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<DateTime<Utc>, _>("last_seen_at")))
}
