//! WIP-limit queries (Cluster 362, G11): the per-workspace claim cap
//! (`maidan_wip_limits`) plus a member's live-claim count over `maidan_threads`.

use maidan_types::{MemberId, WorkspaceId};
use sqlx::{PgPool, Row};

use crate::error::StoreError;

/// Upsert (`Some`) or clear (`None`) the workspace's WIP limit.
pub async fn set_limit(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: Option<i64>,
) -> Result<(), StoreError> {
    match limit {
        Some(n) => {
            sqlx::query(
                "INSERT INTO maidan_wip_limits (workspace_id, wip_limit)
                 VALUES ($1, $2)
                 ON CONFLICT (workspace_id)
                 DO UPDATE SET wip_limit = EXCLUDED.wip_limit, updated_at = NOW()",
            )
            .bind(workspace_id.0)
            .bind(n)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM maidan_wip_limits WHERE workspace_id = $1")
                .bind(workspace_id.0)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// The workspace's WIP limit, or `None` if unset (unlimited).
pub async fn get_limit(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<i64>, StoreError> {
    let row = sqlx::query("SELECT wip_limit FROM maidan_wip_limits WHERE workspace_id = $1")
        .bind(workspace_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<i64, _>("wip_limit")))
}

/// Count a member's live claims — the complement of the `claim_next` claimability
/// predicate (an expired lease is claimable, so it is not live).
pub async fn count_live_claims(pool: &PgPool, member_id: MemberId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM maidan_threads
         WHERE assignee_id = $1 AND tombstoned_at IS NULL
           AND state NOT IN ('closed', 'archived')
           AND (assignment_expires_at IS NULL OR assignment_expires_at >= NOW())",
    )
    .bind(member_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}
