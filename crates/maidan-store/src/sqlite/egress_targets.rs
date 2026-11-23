//! The egress trust boundary. SQLite twin of the Postgres module — timestamps
//! are store-bound rfc3339 text.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{
    validate_allowlist_selector, AllowedEgressTarget, EgressSurface, EgressTargetId,
    NewEgressTarget, WorkspaceId,
};

const COLS: &str = "id, workspace_id, surface, selector, created_at, created_by";

#[cfg(feature = "test-support")]
pub async fn allow(
    pool: &SqlitePool,
    new: NewEgressTarget,
) -> Result<AllowedEgressTarget, StoreError> {
    let mut conn = pool.acquire().await?;
    allow_on(&mut conn, new).await
}

pub(crate) async fn allow_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewEgressTarget,
) -> Result<AllowedEgressTarget, StoreError> {
    validate_allowlist_selector(new.surface, &new.selector)
        .map_err(|why| StoreError::InvalidInput(why.to_string()))?;
    // The server's own credentials serve only a workspace its operator opened
    // (`instance_credentials`). Checked here, on the connection that writes,
    // so the audited and the plain blessing cannot differ.
    let origin = super::import::origin_on(&mut *conn, new.workspace_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    crate::instance_credentials::ensure_served(origin)?;
    let id = EgressTargetId::new();
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_egress_targets (id, workspace_id, surface, selector, created_at, created_by)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, surface, selector) DO UPDATE
           SET selector = excluded.selector
         RETURNING {COLS}"
    ))
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.surface.as_str())
    .bind(&new.selector)
    .bind(&now)
    .bind(new.created_by.map(|m| m.0))
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_target(&row))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<AllowedEgressTarget>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_egress_targets
         WHERE workspace_id = ?
         ORDER BY surface ASC, selector ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_target).collect())
}

#[cfg(feature = "test-support")]
pub async fn revoke(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: EgressTargetId,
) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    revoke_on(&mut conn, workspace_id, id).await
}

pub(crate) async fn revoke_on(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
    id: EgressTargetId,
) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_egress_targets WHERE id = ? AND workspace_id = ?")
        .bind(id.0)
        .bind(workspace_id.0)
        .execute(&mut *conn)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Whether the destination is blessed for a workspace the server's own
/// credentials serve. An entry a sign-up or sandbox workspace made before
/// `instance_credentials` refused them counts for nothing.
pub async fn is_allowed(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    surface: EgressSurface,
    selector: &str,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS present FROM maidan_egress_targets
         WHERE workspace_id = ? AND surface = ? AND selector = ?
           AND EXISTS (SELECT 1 FROM maidan_workspaces w
                       WHERE w.id = maidan_egress_targets.workspace_id
                         AND w.origin = 'operator')",
    )
    .bind(workspace_id.0)
    .bind(surface.as_str())
    .bind(selector)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

fn row_to_target(row: &sqlx::sqlite::SqliteRow) -> AllowedEgressTarget {
    let surface: String = row.get("surface");
    let selector: String = row.get("selector");
    AllowedEgressTarget {
        id: EgressTargetId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        dead: AllowedEgressTarget::dead_reason(&surface, &selector),
        surface,
        selector,
        created_at: row.get("created_at"),
        created_by: row
            .get::<Option<uuid::Uuid>, _>("created_by")
            .map(maidan_types::MemberId),
    }
}
