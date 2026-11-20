use maidan_types::{ContentKeyring, WorkspaceEraseResult, WorkspaceId};
use sqlx::PgConnection;
#[cfg(feature = "test-support")]
use sqlx::PgPool;
use sqlx::Row;

use crate::error::StoreError;
use crate::postgres::purge_workspace;

#[cfg(feature = "test-support")]
pub async fn erase(
    pool: &PgPool,
    keys: &ContentKeyring,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceEraseResult, StoreError> {
    let mut conn = pool.acquire().await?;
    erase_on(&mut conn, keys, workspace_id).await
}

/// The purge and the workspace row's deletion commit together; a held
/// workspace is refused inside the same transaction.
pub(crate) async fn erase_on(
    conn: &mut PgConnection,
    keys: &ContentKeyring,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceEraseResult, StoreError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let purge = purge_workspace::purge_on(&mut tx, keys, workspace_id).await?;
    delete_uncascaded_references(&mut tx, workspace_id).await?;
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

/// The foreign keys that neither cascade nor null on delete, pointing at one
/// table: the referencing table, its column, and the column it names.
const UNCASCADED_REFERENCES: &str = "
    SELECT c.conrelid::regclass::text AS child,
           quote_ident(a.attname) AS child_column,
           quote_ident(pa.attname) AS parent_column
    FROM pg_constraint c
    JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
    JOIN pg_attribute pa ON pa.attrelid = c.confrelid AND pa.attnum = c.confkey[1]
    WHERE c.contype = 'f'
      AND c.confdeltype IN ('a', 'r')
      AND c.confrelid = $1::regclass
      AND array_length(c.conkey, 1) = 1
      AND c.conparentid = 0
    ORDER BY 1, 2";

/// How far the walk below follows references to references. The schema's
/// longest chain is two (a token names a delegation grant, which names a
/// member); the bound only stops a cycle.
const REFERENCE_DEPTH: usize = 8;

/// Deletes every row that names one of the workspace's members through a
/// foreign key that does not cascade (who made a transition, who produced a
/// result, who reported usage, who sent an invite), and the rows that name
/// those rows, deepest first.
///
/// Deleting the workspace cascades to its members before it reaches its
/// channels, and Postgres checks a `NO ACTION` key when that nested delete
/// ends, not when the whole statement does. So a member that a thread's
/// history still names failed the erase of any workspace that had done work.
/// The keys are read from the catalog, not listed here, so a table added
/// later with such a key cannot break the erase again.
async fn delete_uncascaded_references(
    tx: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<(), StoreError> {
    // A table and the predicate that selects its rows going with the workspace
    let mut doomed: Vec<(String, String)> =
        vec![("maidan_members".to_owned(), "workspace_id = $1".to_owned())];
    let mut level = 0;
    for _ in 0..REFERENCE_DEPTH {
        let end = doomed.len();
        let frontier = doomed[level..end].to_vec();
        for (parent, predicate) in frontier {
            let references = sqlx::query(UNCASCADED_REFERENCES)
                .bind(&parent)
                .fetch_all(&mut *tx)
                .await?;
            for row in references {
                let child: String = row.get("child");
                let child_column: String = row.get("child_column");
                let parent_column: String = row.get("parent_column");
                doomed.push((
                    child,
                    format!(
                        "{child_column} IN (SELECT {parent_column} FROM {parent} WHERE {predicate})"
                    ),
                ));
            }
        }
        if doomed.len() == end {
            break;
        }
        level = end;
    }
    for (table, predicate) in doomed.iter().skip(1).rev() {
        sqlx::query(&format!("DELETE FROM {table} WHERE {predicate}"))
            .bind(workspace_id.0)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}
