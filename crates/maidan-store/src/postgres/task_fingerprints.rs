//! Task fingerprints on Postgres (Context Economics C8): stored with the
//! thread, linked to the oldest open task in the workspace with the same
//! fingerprint.

use maidan_types::{TaskAttributes, TaskFingerprint, Thread, ThreadId};
use sqlx::{PgPool, Row};
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
    conn: &mut sqlx::PgConnection,
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
        sqlx::query_scalar("SELECT workspace_id FROM maidan_channels WHERE id = $1")
            .bind(thread.channel_id.0)
            .fetch_one(&mut *conn)
            .await?;
    let issue = attributes
        .linear_issue
        .as_deref()
        .filter(|_| attributes.kind == maidan_types::TaskKind::Change);
    insert_on(conn, thread.id, workspace_id, &fingerprint, issue).await
}

/// Store `fingerprint` for the new `thread_id` inside the create
/// transaction, linked to the oldest live task in `workspace_id` that is
/// open or in review and has the same fingerprint. Two identical tasks
/// created at the same moment may both find nothing: the link is a hint for
/// the creator and the claimant, not a uniqueness rule.
async fn insert_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
    workspace_id: Uuid,
    fingerprint: &str,
    linear_issue: Option<&str>,
) -> Result<Option<ThreadId>, StoreError> {
    let same_brief: Option<Uuid> = sqlx::query_scalar(
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
    // No identical brief: a change thread for the same Linear issue is still
    // a likely duplicate, even under another title (C8 follow-up).
    let duplicate_of = match (same_brief, linear_issue) {
        (Some(id), _) => Some(id),
        (None, Some(issue)) => {
            sqlx::query_scalar(
                "SELECT t.id
                 FROM maidan_thread_attributes a
                 JOIN maidan_threads t ON t.id = a.thread_id
                 JOIN maidan_channels ch ON ch.id = t.channel_id
                 WHERE ch.workspace_id = $1 AND a.linear_issue = $2 AND a.kind = 'change'
                   AND t.id <> $3
                   AND t.tombstoned_at IS NULL AND t.state IN ('open', 'in_review')
                 ORDER BY t.created_at, t.id
                 LIMIT 1",
            )
            .bind(workspace_id)
            .bind(issue)
            .bind(thread_id.0)
            .fetch_optional(&mut *conn)
            .await?
        }
        (None, None) => None,
    };
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

/// The live original `thread_id` duplicates, if any: the task its
/// fingerprint was linked to, while that task is still open or in review.
/// Mirrors [`crate::claimability::DUPLICATE`].
pub(crate) async fn live_original_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
) -> Result<Option<ThreadId>, StoreError> {
    let original: Option<Uuid> = sqlx::query_scalar(
        "SELECT orig.id FROM maidan_task_fingerprints fp
         JOIN maidan_threads orig ON orig.id = fp.duplicate_of
         WHERE fp.thread_id = $1 AND orig.tombstoned_at IS NULL
           AND orig.state IN ('open', 'in_review')",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(original.map(ThreadId))
}

/// Refuse a claim on a live duplicate. The original is not named: the
/// claimer may not be able to read it, and the fingerprint route shows it to
/// those who can.
pub(crate) async fn refuse_duplicate_claim_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    if live_original_on(conn, thread_id).await?.is_some() {
        return Err(StoreError::Conflict(
            "this task duplicates an open task: reuse that one (GET /threads/{id}/fingerprint names it), or clear the link to claim this one".into(),
        ));
    }
    Ok(())
}

/// Clear `thread_id`'s duplicate link, so it is claimable again. `NotFound`
/// when the thread has no fingerprint. Idempotent.
pub async fn clear_duplicate(pool: &PgPool, thread_id: ThreadId) -> Result<(), StoreError> {
    let done =
        sqlx::query("UPDATE maidan_task_fingerprints SET duplicate_of = NULL, cleared_at = now() WHERE thread_id = $1")
            .bind(thread_id.0)
            .execute(pool)
            .await?;
    if done.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}
