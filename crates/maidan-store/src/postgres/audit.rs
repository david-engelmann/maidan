use chrono::{DateTime, Utc};
use maidan_types::{AuditEvent, MemberId, NewAuditEvent, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn append(pool: &PgPool, new: NewAuditEvent) -> Result<AuditEvent, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_audit (actor_id, action, target_kind, target_id, metadata)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, occurred_at, actor_id, action, target_kind, target_id, metadata",
    )
    .bind(new.actor_id.map(|m| m.0))
    .bind(&new.action)
    .bind(new.target_kind.as_deref())
    .bind(new.target_id)
    .bind(&new.metadata)
    .fetch_one(pool)
    .await?;
    Ok(row_to_audit(&row))
}

pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<AuditEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, occurred_at, actor_id, action, target_kind, target_id, metadata
         FROM maidan_audit a
         WHERE (a.target_kind = 'workspace' AND a.target_id = $1)
            OR a.actor_id IN (
              SELECT m.id FROM maidan_members m WHERE m.workspace_id = $1
            )
         ORDER BY a.occurred_at DESC, a.id DESC
         LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_audit).collect())
}

pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<AuditEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, occurred_at, actor_id, action, target_kind, target_id, metadata
         FROM maidan_audit
         ORDER BY occurred_at DESC, id DESC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_audit).collect())
}

fn row_to_audit(row: &sqlx::postgres::PgRow) -> AuditEvent {
    AuditEvent {
        id: row.get("id"),
        occurred_at: row.get::<DateTime<Utc>, _>("occurred_at"),
        actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
        action: row.get("action"),
        target_kind: row.get("target_kind"),
        target_id: row.get("target_id"),
        metadata: row.get::<serde_json::Value, _>("metadata"),
    }
}
