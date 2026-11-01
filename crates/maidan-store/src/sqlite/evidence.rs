//! Evidence a reviewer is shown: a thread's version, which the database
//! bumps on every write to the thread's content (migration 0144), and the
//! artifacts linked to the thread, and the evidence records attached to it.

use chrono::{DateTime, Utc};
use maidan_types::{
    Event, EvidenceRecord, EvidenceRecordKind, MemberId, NewEvidenceRecord, StoredEvent,
    ThreadArtifact, ThreadId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

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
    .bind(crate::attribution::delegate_acting_for(linked_by).unwrap_or(linked_by).0)
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

fn row_to_record(row: &sqlx::sqlite::SqliteRow) -> Result<EvidenceRecord, StoreError> {
    let kind: String = row.get("kind");
    Ok(EvidenceRecord {
        id: row.get::<Uuid, _>("id"),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        kind: EvidenceRecordKind::parse(&kind)
            .ok_or_else(|| StoreError::InvalidInput(format!("stored evidence record kind {kind}")))?,
        source: row.get("source"),
        url: row.get("url"),
        sha256: row.get("sha256"),
        note: row.get("note"),
        recorded_by: MemberId(row.get::<Uuid, _>("recorded_by")),
        recorded_at: row.get::<DateTime<Utc>, _>("recorded_at"),
    })
}

/// Attach an evidence record to a thread and append `evidence_added` in the
/// same transaction. `record` is already normalized by the caller.
/// `NotFound` for no such thread. Records are not part of the thread
/// version: adding one does not move it.
pub async fn add_record_with_event(
    pool: &SqlitePool,
    thread_id: ThreadId,
    record: &NewEvidenceRecord,
    recorded_by: MemberId,
) -> Result<(EvidenceRecord, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
    let stored = EvidenceRecord {
        id: Uuid::new_v4(),
        thread_id,
        kind: record.kind,
        source: record.source.clone(),
        url: record.url.clone(),
        sha256: record.sha256.clone(),
        note: record.note.clone(),
        recorded_by,
        recorded_at: Utc::now(),
    };
    sqlx::query(
        "INSERT INTO maidan_thread_evidence_records
             (id, thread_id, kind, source, url, sha256, note, recorded_by, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(stored.id)
    .bind(thread_id.0)
    .bind(stored.kind.as_str())
    .bind(&stored.source)
    .bind(&stored.url)
    .bind(&stored.sha256)
    .bind(&stored.note)
    .bind(recorded_by.0)
    .bind(stored.recorded_at)
    .execute(&mut *tx)
    .await?;
    let event = Event::EvidenceAdded {
        occurred_at: stored.recorded_at,
        workspace_id,
        channel_id,
        thread_id,
        record: stored.clone(),
    };
    let appended = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((stored, appended))
}

/// A thread's evidence records, oldest first.
pub async fn list_records(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<EvidenceRecord>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, thread_id, kind, source, url, sha256, note, recorded_by, recorded_at
         FROM maidan_thread_evidence_records
         WHERE thread_id = ?1
         ORDER BY recorded_at, id",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_record).collect()
}
