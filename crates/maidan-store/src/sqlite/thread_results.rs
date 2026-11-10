use chrono::{DateTime, Utc};
use maidan_types::{
    result_kind_from_payload, ChannelClosedResult, ChannelId, ClaimLeaseId, MemberId, ThreadId,
    ThreadResult, ThreadState, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a task's structured result. A re-set overwrites the prior
/// result. JSON is stored as TEXT in SQLite. `result_kind` is extracted from
/// the payload — a namespaced string, not an enum.
pub async fn set(
    pool: &SqlitePool,
    grace: &crate::claim_grace::ClaimGrace,
    thread_id: ThreadId,
    produced_by: MemberId,
    lease_id: ClaimLeaseId,
    result: &serde_json::Value,
) -> Result<ThreadResult, StoreError> {
    let result_text = serde_json::to_string(result)?;
    let result_kind = result_kind_from_payload(result);
    let clock = Utc::now();
    let now = clock.to_rfc3339();
    // A review's holder is judged again before its verdict is stored, on its
    // usage since and on what the result says served it: every claim-time
    // check ran on the usage it had reported then.
    super::review_family::refuse_result(pool, thread_id, produced_by, lease_id, result).await?;
    // The claim fence is the SELECT: no row unless `produced_by` holds the
    // thread under `lease_id` right now, in the same statement as the write.
    // A lease that expired is lost even before the reaper frees it — see the
    // Postgres twin. SQLite has one writer, so no takeover can interleave.
    let row = sqlx::query(
        "INSERT INTO maidan_thread_results
             (thread_id, result, produced_by, produced_at, result_kind, produced_actor_id)
         SELECT t.id, ?, ?, ?, ?, ?
         FROM maidan_threads t
         WHERE t.id = ? AND t.assignee_id = ? AND t.claim_lease_id = ?
           AND t.tombstoned_at IS NULL
           AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             result = excluded.result,
             produced_by = excluded.produced_by,
             produced_at = excluded.produced_at,
             result_kind = excluded.result_kind,
             produced_actor_id = excluded.produced_actor_id
         RETURNING thread_id, result, produced_by, produced_at",
    )
    .bind(&result_text)
    .bind(produced_by.0)
    .bind(&now)
    .bind(result_kind)
    // The delegate that carried a borrowed token, so a hand-off can tell a
    // worker's result from one a non-worker produced.
    .bind(
        crate::attribution::delegate_acting_for(produced_by)
            .unwrap_or(produced_by)
            .0,
    )
    .bind(thread_id.0)
    .bind(produced_by.0)
    .bind(lease_id.0)
    .bind(grace.lease_clock(clock).to_rfc3339())
    .fetch_optional(pool)
    .await?;
    match row {
        Some(row) => row_to_result(&row),
        None => Err(super::threads::fence_refusal(pool, thread_id, lease_id).await?),
    }
}

/// A task's result, or `None` if none has been produced.
pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadResult>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, result, produced_by, produced_at
         FROM maidan_thread_results WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_result).transpose()
}

fn row_to_result(row: &sqlx::sqlite::SqliteRow) -> Result<ThreadResult, StoreError> {
    let result_text: String = row.get("result");
    Ok(ThreadResult {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        result: serde_json::from_str(&result_text)?,
        produced_by: MemberId(row.get::<Uuid, _>("produced_by")),
        produced_at: row.get::<DateTime<Utc>, _>("produced_at"),
    })
}

/// Workspace-scoped result list. Exact-match on the extracted `result_kind`
/// when `Some`; `None`/empty is unfiltered. Tombstoned threads are dropped.
/// `limit` is clamped `1..=500`.
pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    result_kind: Option<&str>,
    limit: i64,
) -> Result<Vec<ThreadResult>, StoreError> {
    let limit = limit.clamp(1, 500);
    let result_kind = result_kind.map(str::trim).filter(|s| !s.is_empty());
    let rows = sqlx::query(
        "SELECT tr.thread_id AS thread_id, tr.result AS result,
                tr.produced_by AS produced_by, tr.produced_at AS produced_at
         FROM maidan_thread_results tr
         JOIN maidan_threads t ON t.id = tr.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?1
           AND t.tombstoned_at IS NULL
           AND (?2 IS NULL OR tr.result_kind = ?2)
         ORDER BY tr.produced_at DESC, tr.thread_id DESC
         LIMIT ?3",
    )
    .bind(workspace_id.0)
    .bind(result_kind)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_result).collect()
}

/// Closed/archived results in a channel — see the Postgres twin. `result` is
/// TEXT JSON. `limit` is clamped `1..=50`.
pub async fn list_closed_in_channel(
    pool: &SqlitePool,
    channel_id: ChannelId,
    exclude_thread_id: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<ChannelClosedResult>, StoreError> {
    let limit = limit.clamp(1, 50);
    let rows = sqlx::query(
        "SELECT tr.thread_id AS thread_id, t.title AS title, t.state AS state,
                tr.result AS result, tr.produced_by AS produced_by,
                tr.produced_at AS produced_at
         FROM maidan_thread_results tr
         JOIN maidan_threads t ON t.id = tr.thread_id
         WHERE t.channel_id = ?1
           AND t.state IN ('closed', 'archived')
           AND t.tombstoned_at IS NULL
           AND (?2 IS NULL OR t.id <> ?2)
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_cancellations x WHERE x.thread_id = t.id)
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_unlanded_closes u WHERE u.thread_id = t.id)
         ORDER BY tr.produced_at DESC, tr.thread_id DESC
         LIMIT ?3",
    )
    .bind(channel_id.0)
    .bind(exclude_thread_id.map(|id| id.0))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_closed).collect()
}

fn row_to_closed(row: &sqlx::sqlite::SqliteRow) -> Result<ChannelClosedResult, StoreError> {
    let state_str: String = row.get("state");
    let state = ThreadState::parse(&state_str)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown thread state: {state_str}")))?;
    let result_text: String = row.get("result");
    Ok(ChannelClosedResult {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        title: row.get("title"),
        state,
        result: serde_json::from_str(&result_text)?,
        produced_by: MemberId(row.get::<Uuid, _>("produced_by")),
        produced_at: row.get::<DateTime<Utc>, _>("produced_at"),
    })
}
