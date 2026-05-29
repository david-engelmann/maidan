use maidan_types::{WorkspaceEraseResult, WorkspaceId};
use sqlx::PgPool;

use crate::error::StoreError;
use crate::postgres::{purge_workspace, workspaces};

pub async fn erase(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceEraseResult, StoreError> {
    workspaces::get(pool, workspace_id).await?;
    let purge = purge_workspace::purge(pool, workspace_id).await?;
    let deleted = sqlx::query("DELETE FROM maidan_workspaces WHERE id = $1")
        .bind(workspace_id.0)
        .execute(pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(WorkspaceEraseResult {
        purge,
        workspace_erased: true,
    })
}
