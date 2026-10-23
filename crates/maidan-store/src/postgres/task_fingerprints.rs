//! Task fingerprints on Postgres (Context Economics C8): stored with the
//! thread, linked to the oldest open task in the workspace with the same
//! fingerprint.

use maidan_types::{TaskFingerprint, ThreadId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Store `fingerprint` for the new `thread_id` inside the create
/// transaction, linked to the oldest live task in `workspace_id` that is
/// open or in review and has the same fingerprint. Two identical tasks
/// created at the same moment may both find nothing: the link is a hint for
/// the creator and the claimant, not a uniqueness rule.
pub(crate) async fn insert_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
    workspace_id: Uuid,
    fingerprint: &str,
) -> Result<Option<ThreadId>, StoreError> {
    let duplicate_of: Option<Uuid> = sqlx::query_scalar(
        "SELECT f.thread_id
         FROM maidan_task_fingerprints f
         JOIN maidan_threads t ON t.id = f.thread_id
         WHERE f.workspace_id = $1 AND f.fingerprint = $2 AND f.thread_id <> $3
           AND t.tombstoned_at IS NULL AND t.state IN ('open', 'in_review')
         ORDER BY f.created_at, f.thread_id
         LIMIT 1",
    )
    .bind(workspace_id)
    .bind(fingerprint)
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO maidan_task_fingerprints (thread_id, workspace_id, fingerprint, duplicate_of)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(thread_id.0)
    .bind(workspace_id)
    .bind(fingerprint)
    .bind(duplicate_of)
    .execute(&mut *conn)
    .await?;
    Ok(duplicate_of.map(ThreadId))
}

/// A thread's stored fingerprint; `None` for a thread created without a
/// brief or before fingerprints existed.
pub async fn get(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Option<TaskFingerprint>, StoreError> {
    let row = sqlx::query(
        "SELECT fingerprint, duplicate_of FROM maidan_task_fingerprints WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        Ok(TaskFingerprint {
            fingerprint: row.try_get("fingerprint")?,
            duplicate_of: row
                .try_get::<Option<Uuid>, _>("duplicate_of")?
                .map(ThreadId),
        })
    })
    .transpose()
}
