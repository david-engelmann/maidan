use chrono::{DateTime, Utc};
use maidan_types::MemberId;
use sqlx::{PgPool, Row};

use crate::error::StoreError;

/// Record that a member was just seen (Cluster 252) — see the SQLite twin.
pub async fn touch(pool: &PgPool, member_id: MemberId) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_member_last_seen (member_id, last_seen_at)
         VALUES ($1, now())
         ON CONFLICT (member_id) DO UPDATE SET last_seen_at = now()",
    )
    .bind(member_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &PgPool, member_id: MemberId) -> Result<Option<DateTime<Utc>>, StoreError> {
    let row = sqlx::query("SELECT last_seen_at FROM maidan_member_last_seen WHERE member_id = $1")
        .bind(member_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<DateTime<Utc>, _>("last_seen_at")))
}
