use chrono::{DateTime, Utc};
use maidan_types::{AuditEvent, DelegationGrantId, MemberId, NewAuditEvent, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Every audit read selects these, in this order — see the SQLite twin.
const AUDIT_COLUMNS: &str =
    "id, occurred_at, actor_id, subject_id, grant_id, action, target_kind, target_id, metadata";

pub async fn append(pool: &PgPool, new: NewAuditEvent) -> Result<AuditEvent, StoreError> {
    let mut conn = pool.acquire().await?;
    append_on(&mut conn, new).await
}

/// [`append`] on the caller's connection — inside a transaction, the audit row
/// commits or rolls back with the change it records (D-A).
pub(crate) async fn append_on(
    conn: &mut sqlx::PgConnection,
    new: NewAuditEvent,
) -> Result<AuditEvent, StoreError> {
    let (actor, subject, grant) = crate::attribution::audit_principal(new.actor_id);
    let sql = format!(
        "INSERT INTO maidan_audit
            (actor_id, subject_id, grant_id, action, target_kind, target_id, metadata)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {AUDIT_COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(actor.map(|m| m.0))
        .bind(subject.map(|m| m.0))
        .bind(grant.map(|g| g.0))
        .bind(&new.action)
        .bind(new.target_kind.as_deref())
        .bind(new.target_id)
        .bind(&new.metadata)
        .fetch_one(&mut *conn)
        .await?;
    Ok(row_to_audit(&row))
}

pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<AuditEvent>, StoreError> {
    let sql = format!(
        "SELECT {AUDIT_COLUMNS}
         FROM maidan_audit a
         WHERE (a.target_kind = 'workspace' AND a.target_id = $1)
            OR a.actor_id IN (
              SELECT m.id FROM maidan_members m WHERE m.workspace_id = $1
            )
         ORDER BY a.occurred_at DESC, a.id DESC
         LIMIT $2"
    );
    let rows = sqlx::query(&sql)
        .bind(workspace_id.0)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(row_to_audit).collect())
}

pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<AuditEvent>, StoreError> {
    let sql = format!(
        "SELECT {AUDIT_COLUMNS}
         FROM maidan_audit
         ORDER BY occurred_at DESC, id DESC
         LIMIT $1"
    );
    let rows = sqlx::query(&sql).bind(limit).fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_audit).collect())
}

fn row_to_audit(row: &sqlx::postgres::PgRow) -> AuditEvent {
    AuditEvent {
        id: row.get("id"),
        occurred_at: row.get::<DateTime<Utc>, _>("occurred_at"),
        actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
        subject_id: row.get::<Option<Uuid>, _>("subject_id").map(MemberId),
        grant_id: row
            .get::<Option<Uuid>, _>("grant_id")
            .map(DelegationGrantId),
        action: row.get("action"),
        target_kind: row.get("target_kind"),
        target_id: row.get("target_id"),
        metadata: row.get::<serde_json::Value, _>("metadata"),
    }
}
