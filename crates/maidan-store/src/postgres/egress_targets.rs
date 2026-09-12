//! The egress trust boundary (Cluster 378.1): a per-workspace allowlist of the
//! external destinations Maidan may deliver to. See the SQLite twin.
//!
//! A result's `deliver_to` list is agent-written; Maidan's connector credentials
//! are operator-held and reach far more than one repository. So `deliver_to`
//! *selects* and this allowlist *authorizes*, and an empty allowlist authorizes
//! nothing — the Cluster-371 secret-broker fail-safe.

use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{
    validate_allowlist_selector, AllowedEgressTarget, EgressSurface, EgressTargetId,
    NewEgressTarget, WorkspaceId,
};

const COLS: &str = "id, workspace_id, surface, selector, created_at";

/// Bless a destination. Idempotent: re-blessing returns the existing entry
/// (original `created_at` intact) rather than adding a second row, so the list
/// stays the operator's "what may we post to" with nothing to reconcile.
pub async fn allow(pool: &PgPool, new: NewEgressTarget) -> Result<AllowedEgressTarget, StoreError> {
    validate_allowlist_selector(new.surface, &new.selector)
        .map_err(|why| StoreError::InvalidInput(why.to_string()))?;
    let id = EgressTargetId::new();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_egress_targets (id, workspace_id, surface, selector, created_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (workspace_id, surface, selector) DO UPDATE
           SET selector = maidan_egress_targets.selector
         RETURNING {COLS}"
    ))
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.surface.as_str())
    .bind(&new.selector)
    .fetch_one(pool)
    .await?;
    Ok(row_to_target(&row))
}

pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<AllowedEgressTarget>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_egress_targets
         WHERE workspace_id = $1
         ORDER BY surface ASC, selector ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_target).collect())
}

/// Revoke a blessing. Scoped to the workspace, so one workspace's admin cannot
/// revoke another's entry by guessing an id. `false` when no such entry exists.
pub async fn revoke(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    id: EgressTargetId,
) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_egress_targets WHERE id = $1 AND workspace_id = $2")
        .bind(id.0)
        .bind(workspace_id.0)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// The authorization check itself. `selector` is the *allowlist* grain — for
/// GitHub the repository, not `owner/name#123` — so callers pass
/// `EgressTarget::allowlist_selector()`, never `selector()`.
pub async fn is_allowed(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    surface: EgressSurface,
    selector: &str,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS present FROM maidan_egress_targets
         WHERE workspace_id = $1 AND surface = $2 AND selector = $3",
    )
    .bind(workspace_id.0)
    .bind(surface.as_str())
    .bind(selector)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

fn row_to_target(row: &sqlx::postgres::PgRow) -> AllowedEgressTarget {
    AllowedEgressTarget {
        id: EgressTargetId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        surface: row.get("surface"),
        selector: row.get("selector"),
        created_at: row.get("created_at"),
    }
}
