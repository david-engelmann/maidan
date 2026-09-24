use chrono::{DateTime, Utc};
use maidan_types::{AuditEvent, DelegationGrantId, MemberId, NewAuditEvent, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Every audit read selects these, in this order, so a new column cannot be
/// missed by one query and silently read as `None`.
const AUDIT_COLUMNS: &str =
    "id, occurred_at, actor_id, subject_id, grant_id, action, target_kind, target_id, metadata";

pub async fn append(pool: &SqlitePool, new: NewAuditEvent) -> Result<AuditEvent, StoreError> {
    let mut conn = pool.acquire().await?;
    append_on(&mut conn, new).await
}

/// [`append`] on the caller's connection — inside a transaction, the audit row
/// commits or rolls back with the change it records (D-A).
pub(crate) async fn append_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewAuditEvent,
) -> Result<AuditEvent, StoreError> {
    let now = Utc::now();
    let metadata_text = serde_json::to_string(&new.metadata)?;
    let (actor, subject, grant) = crate::attribution::audit_principal(new.actor_id);
    let sql = format!(
        "INSERT INTO maidan_audit
            (occurred_at, actor_id, subject_id, grant_id, action, target_kind, target_id, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING {AUDIT_COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(now)
        .bind(actor.map(|m| m.0))
        .bind(subject.map(|m| m.0))
        .bind(grant.map(|g| g.0))
        .bind(&new.action)
        .bind(new.target_kind.as_deref())
        .bind(new.target_id)
        .bind(&metadata_text)
        .fetch_one(&mut *conn)
        .await?;
    row_to_audit(&row)
}

pub async fn list_for_workspace(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<AuditEvent>, StoreError> {
    let sql = format!(
        "SELECT {AUDIT_COLUMNS}
         FROM maidan_audit a
         WHERE (a.target_kind = 'workspace' AND a.target_id = ?)
            OR a.actor_id IN (
              SELECT m.id FROM maidan_members m WHERE m.workspace_id = ?
            )
         ORDER BY a.occurred_at DESC, a.id DESC
         LIMIT ?"
    );
    let rows = sqlx::query(&sql)
        .bind(workspace_id.0)
        .bind(workspace_id.0)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter().map(row_to_audit).collect()
}

pub async fn list(pool: &SqlitePool, limit: i64) -> Result<Vec<AuditEvent>, StoreError> {
    let sql = format!(
        "SELECT {AUDIT_COLUMNS}
         FROM maidan_audit
         ORDER BY occurred_at DESC, id DESC
         LIMIT ?"
    );
    let rows = sqlx::query(&sql).bind(limit).fetch_all(pool).await?;
    rows.iter().map(row_to_audit).collect()
}

fn row_to_audit(row: &sqlx::sqlite::SqliteRow) -> Result<AuditEvent, StoreError> {
    let metadata_text: String = row.get("metadata");
    let metadata = serde_json::from_str(&metadata_text)?;
    Ok(AuditEvent {
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
        metadata,
    })
}
