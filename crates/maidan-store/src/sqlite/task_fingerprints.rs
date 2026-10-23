//! Task fingerprints on SQLite; see the Postgres twin.

use chrono::Utc;
use maidan_types::{TaskAttributes, TaskFingerprint, Thread, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Fingerprint a just-inserted `thread` from its brief and the task
/// `attributes` it was created with, and store it in the same transaction.
/// Every thread-creating path calls this (plain create, create with
/// footprint, plan children including review children, recipe threads), so
/// a fingerprint never depends on which door a task came in by. Imported
/// history is the one exception: it restores past threads, not new work.
/// A thread with no brief stores nothing.
pub(crate) async fn record_on(
    conn: &mut sqlx::SqliteConnection,
    thread: &Thread,
    attributes: &TaskAttributes,
) -> Result<Option<ThreadId>, StoreError> {
    let Some(fingerprint) = maidan_types::task_fingerprint(
        thread.title.as_deref(),
        thread.description.as_deref(),
        attributes,
    ) else {
        return Ok(None);
    };
    let workspace_id: Uuid =
        sqlx::query_scalar("SELECT workspace_id FROM maidan_channels WHERE id = ?")
            .bind(thread.channel_id.0)
            .fetch_one(&mut *conn)
            .await?;
    insert_on(conn, thread.id, workspace_id, &fingerprint).await
}

/// Store `fingerprint` for the new `thread_id` inside the create
/// transaction, linked to the oldest open task with the same fingerprint;
/// see the Postgres twin.
async fn insert_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
    workspace_id: Uuid,
    fingerprint: &str,
) -> Result<Option<ThreadId>, StoreError> {
    let duplicate_of: Option<Uuid> = sqlx::query_scalar(
        "SELECT f.thread_id
         FROM maidan_task_fingerprints f
         JOIN maidan_threads t ON t.id = f.thread_id
         WHERE f.workspace_id = ?1 AND f.fingerprint = ?2 AND f.thread_id <> ?3
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
        "INSERT INTO maidan_task_fingerprints (thread_id, workspace_id, fingerprint, duplicate_of, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(thread_id.0)
    .bind(workspace_id)
    .bind(fingerprint)
    .bind(duplicate_of)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *conn)
    .await?;
    Ok(duplicate_of.map(ThreadId))
}

/// A thread's stored fingerprint; see the Postgres twin.
pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<TaskFingerprint>, StoreError> {
    let row = sqlx::query(
        "SELECT fingerprint, duplicate_of FROM maidan_task_fingerprints WHERE thread_id = ?1",
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
