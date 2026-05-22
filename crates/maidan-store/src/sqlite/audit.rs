use chrono::{DateTime, Utc};
use maidan_types::{AuditEvent, MemberId, NewAuditEvent};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn append(pool: &SqlitePool, new: NewAuditEvent) -> Result<AuditEvent, StoreError> {
    let now = Utc::now();
    let metadata_text = serde_json::to_string(&new.metadata)?;
    let row = sqlx::query(
        "INSERT INTO maidan_audit (occurred_at, actor_id, action, target_kind, target_id, metadata)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id, occurred_at, actor_id, action, target_kind, target_id, metadata",
    )
    .bind(now)
    .bind(new.actor_id.map(|m| m.0))
    .bind(&new.action)
    .bind(new.target_kind.as_deref())
    .bind(new.target_id)
    .bind(&metadata_text)
    .fetch_one(pool)
    .await?;
    row_to_audit(&row)
}

pub async fn list(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, occurred_at, actor_id, action, target_kind, target_id, metadata
         FROM maidan_audit
         ORDER BY occurred_at DESC, id DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_audit).collect()
}

fn row_to_audit(row: &sqlx::sqlite::SqliteRow) -> Result<AuditEvent, StoreError> {
    let metadata_text: String = row.get("metadata");
    let metadata = serde_json::from_str(&metadata_text)?;
    Ok(AuditEvent {
        id: row.get("id"),
        occurred_at: row.get::<DateTime<Utc>, _>("occurred_at"),
        actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
        action: row.get("action"),
        target_kind: row.get("target_kind"),
        target_id: row.get("target_id"),
        metadata,
    })
}
