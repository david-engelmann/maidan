use chrono::{DateTime, SecondsFormat, Utc};
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, Event, MemberId, NewApprovalGate, StoredEvent,
    ThreadId, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::a2a::PendingGateQuery;
use crate::error::StoreError;
use crate::thread_access::readable_row;

const GATE_COLUMNS: &str = "id, workspace_id, thread_id, requested_by, prompt, schema, state, \
     content, resolved_by, requested_actor_id, resolved_actor_id, created_at, resolved_at";

/// The `created_at` text form: always millisecond `...Z`, so rows and cursors
/// compare correctly as strings.
fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Open a new `Pending` approval gate. JSON columns are stored as TEXT in
/// SQLite.
pub async fn create(pool: &SqlitePool, gate: &NewApprovalGate) -> Result<ApprovalGate, StoreError> {
    let mut tx = pool.begin().await?;
    let gate = create_in_tx(&mut tx, gate).await?;
    tx.commit().await?;
    Ok(gate)
}

async fn create_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    gate: &NewApprovalGate,
) -> Result<ApprovalGate, StoreError> {
    let id = ApprovalGateId::new();
    let schema_text = gate
        .schema
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let now = ts(Utc::now());
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_approval_gates
             (id, workspace_id, thread_id, requested_by, prompt, schema, state, created_at,
              requested_actor_id)
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(id.0)
    .bind(gate.workspace_id.0)
    .bind(gate.thread_id.map(|t| t.0))
    .bind(gate.requested_by.0)
    .bind(&gate.prompt)
    .bind(schema_text)
    .bind(&now)
    .bind(crate::attribution::delegate_acting_for(gate.requested_by).map(|m| m.0))
    .fetch_one(&mut **tx)
    .await?;
    row_to_gate(&row)
}

pub async fn create_with_event(
    pool: &SqlitePool,
    new: &NewApprovalGate,
) -> Result<(ApprovalGate, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let gate = create_in_tx(&mut tx, new).await?;
    let channel_id = if let Some(thread_id) = gate.thread_id {
        let (workspace_id, channel_id) =
            super::events::thread_scope_in_tx(&mut tx, thread_id).await?;
        if workspace_id != gate.workspace_id {
            return Err(StoreError::InvalidInput(
                "approval gate thread belongs to another workspace".into(),
            ));
        }
        Some(channel_id)
    } else {
        None
    };
    let event = Event::ApprovalRequested {
        occurred_at: gate.created_at,
        workspace_id: gate.workspace_id,
        channel_id,
        thread_id: gate.thread_id,
        gate_id: gate.id,
        requested_by: gate.requested_by,
    };
    let stored = super::events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((gate, stored))
}

pub async fn get(
    pool: &SqlitePool,
    id: ApprovalGateId,
) -> Result<Option<ApprovalGate>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {GATE_COLUMNS} FROM maidan_approval_gates WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_gate).transpose()
}

/// The pending gates in a workspace, oldest first — the queryable held-gate list.
pub async fn list_pending(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<ApprovalGate>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {GATE_COLUMNS} FROM maidan_approval_gates
         WHERE workspace_id = ? AND state = 'pending'
         ORDER BY created_at ASC
         LIMIT ?"
    ))
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_gate).collect()
}

/// The filters `page_pending` and `count_pending` share: `?1` workspace,
/// `?2` thread, `?3` created since, `?4` reader, `?5` the DM channel name.
fn pending_filters() -> String {
    let readable = readable_row("maidan_approval_gates.thread_id", "?1", "?4", "?5");
    format!(
        "workspace_id = ?1 AND state = 'pending'
           AND (?2 IS NULL OR thread_id = ?2)
           AND (?3 IS NULL OR created_at >= ?3)
           AND {readable}"
    )
}

/// A keyset page of the pending gates, newest first. See [`PendingGateQuery`].
pub async fn page_pending(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    query: PendingGateQuery,
) -> Result<Vec<ApprovalGate>, StoreError> {
    let (before_at, before_id) = match query.before {
        Some((at, id)) => (Some(ts(at)), id.map(|id| id.0)),
        None => (None, None),
    };
    let rows = sqlx::query(&format!(
        "SELECT {GATE_COLUMNS} FROM maidan_approval_gates
         WHERE {}
           AND (?6 IS NULL OR created_at < ?6
                OR (created_at = ?6 AND ?7 IS NOT NULL AND id < ?7))
         ORDER BY created_at DESC, id DESC
         LIMIT ?8",
        pending_filters()
    ))
    .bind(workspace_id.0)
    .bind(query.thread_id.map(|t| t.0))
    .bind(query.created_since.map(ts))
    .bind(query.readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .bind(before_at)
    .bind(before_id)
    .bind(query.limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_gate).collect()
}

/// How many pending gates match `query`'s filters.
pub async fn count_pending(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    query: PendingGateQuery,
) -> Result<i64, StoreError> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM maidan_approval_gates WHERE {}",
        pending_filters()
    ))
    .bind(workspace_id.0)
    .bind(query.thread_id.map(|t| t.0))
    .bind(query.created_since.map(ts))
    .bind(query.readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .fetch_one(pool)
    .await?)
}

/// Resolve a `Pending` gate (compare-and-set on `pending` so a double-answer or a
/// late answer after cancel is a no-op → `None`). See the Postgres twin.
pub async fn resolve(
    pool: &SqlitePool,
    id: ApprovalGateId,
    resolved_by: MemberId,
    state: ApprovalGateState,
    content: Option<&serde_json::Value>,
) -> Result<Option<ApprovalGate>, StoreError> {
    let content_text = content.map(serde_json::to_string).transpose()?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "UPDATE maidan_approval_gates
         SET state = ?, content = ?, resolved_by = ?, resolved_at = ?, resolved_actor_id = ?
         WHERE id = ? AND state = 'pending'
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(state.as_str())
    .bind(content_text)
    .bind(resolved_by.0)
    .bind(&now)
    .bind(crate::attribution::delegate_acting_for(resolved_by).map(|m| m.0))
    .bind(id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_gate).transpose()
}

fn row_to_gate(row: &sqlx::sqlite::SqliteRow) -> Result<ApprovalGate, StoreError> {
    let schema_text: Option<String> = row.get("schema");
    let content_text: Option<String> = row.get("content");
    Ok(ApprovalGate {
        id: ApprovalGateId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        thread_id: row.get::<Option<Uuid>, _>("thread_id").map(ThreadId),
        requested_by: MemberId(row.get::<Uuid, _>("requested_by")),
        prompt: row.get::<String, _>("prompt"),
        schema: schema_text.map(|s| serde_json::from_str(&s)).transpose()?,
        state: ApprovalGateState::parse(&row.get::<String, _>("state"))
            .unwrap_or(ApprovalGateState::Pending),
        content: content_text.map(|s| serde_json::from_str(&s)).transpose()?,
        resolved_by: row.get::<Option<Uuid>, _>("resolved_by").map(MemberId),
        requested_actor_id: row
            .get::<Option<Uuid>, _>("requested_actor_id")
            .map(MemberId),
        resolved_actor_id: row
            .get::<Option<Uuid>, _>("resolved_actor_id")
            .map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        resolved_at: row.get::<Option<DateTime<Utc>>, _>("resolved_at"),
    })
}
