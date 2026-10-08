//! Evidence a reviewer is shown: a thread's version, which the database
//! bumps on every write to the thread's content (migration 0144), and the
//! artifacts linked to the thread.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, ThreadArtifact, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_artifact(row: &sqlx::sqlite::SqliteRow) -> ThreadArtifact {
    ThreadArtifact {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        sha256: row.get("sha256"),
        linked_by: MemberId(row.get::<Uuid, _>("linked_by")),
        linked_at: row.get::<DateTime<Utc>, _>("linked_at"),
    }
}

/// The thread's version, 0 before anything was written to its content.
pub async fn version(pool: &SqlitePool, thread_id: ThreadId) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COALESCE(v.version, 0) AS version
         FROM maidan_threads t
         LEFT JOIN maidan_thread_versions v ON v.thread_id = t.id
         WHERE t.id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row.get::<i64, _>("version"))
}

/// Link an artifact to a thread. Only an artifact the thread's workspace holds
/// can be linked: knowing a hash is not access to its bytes. Returns the link
/// and whether it is new; linking twice keeps the first link.
pub async fn link(
    pool: &SqlitePool,
    thread_id: ThreadId,
    sha256: &str,
    linked_by: MemberId,
) -> Result<(ThreadArtifact, bool), StoreError> {
    let mut tx = pool.begin().await?;
    let held = sqlx::query(
        "SELECT 1 FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_artifact_refs r ON r.workspace_id = c.workspace_id AND r.sha256 = ?2
         WHERE t.id = ?1",
    )
    .bind(thread_id.0)
    .bind(sha256)
    .fetch_optional(&mut *tx)
    .await?;
    if held.is_none() {
        return Err(StoreError::NotFound);
    }
    let inserted = sqlx::query(
        "INSERT INTO maidan_thread_artifacts (thread_id, sha256, linked_by, linked_at, linked_actor_id)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (thread_id, sha256) DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(sha256)
    .bind(linked_by.0)
    .bind(Utc::now())
    // A delegate linking with a borrowed token: a hand-off judges it too.
    .bind(crate::attribution::delegate_acting_for(linked_by).map(|m| m.0))
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    let row = sqlx::query(
        "SELECT thread_id, sha256, linked_by, linked_at FROM maidan_thread_artifacts
         WHERE thread_id = ? AND sha256 = ?",
    )
    .bind(thread_id.0)
    .bind(sha256)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((row_to_artifact(&row), inserted))
}

/// Unlink an artifact from a thread. `false` when it was not linked.
pub async fn unlink(
    pool: &SqlitePool,
    thread_id: ThreadId,
    sha256: &str,
) -> Result<bool, StoreError> {
    let removed =
        sqlx::query("DELETE FROM maidan_thread_artifacts WHERE thread_id = ? AND sha256 = ?")
            .bind(thread_id.0)
            .bind(sha256)
            .execute(pool)
            .await?
            .rows_affected();
    Ok(removed > 0)
}

/// The artifacts linked to a thread, in the order they were linked.
pub async fn list(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadArtifact>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, sha256, linked_by, linked_at FROM maidan_thread_artifacts
         WHERE thread_id = ?
         ORDER BY linked_at, sha256",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_artifact).collect())
}
