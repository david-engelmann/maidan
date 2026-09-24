use maidan_types::{WorkspaceEraseResult, WorkspaceId};
use sqlx::{PgConnection, PgPool};

use crate::error::StoreError;
use crate::postgres::purge_workspace;

pub async fn erase(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceEraseResult, StoreError> {
    let mut conn = pool.acquire().await?;
    erase_on(&mut conn, workspace_id).await
}

/// The purge and the workspace row's deletion commit together; a held
/// workspace is refused inside the same transaction.
pub(crate) async fn erase_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceEraseResult, StoreError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let purge = purge_workspace::purge_on(&mut tx, workspace_id).await?;
    let deleted = sqlx::query("DELETE FROM maidan_workspaces WHERE id = $1")
        .bind(workspace_id.0)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    tx.commit().await?;
    Ok(WorkspaceEraseResult {
        purge,
        workspace_erased: true,
    })
}
