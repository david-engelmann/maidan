use chrono::{DateTime, Utc};
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, MemberId, NewApprovalGate, ThreadId,
    WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const GATE_COLUMNS: &str = "id, workspace_id, thread_id, requested_by, prompt, schema, state, \
     content, resolved_by, created_at, resolved_at";

/// Open a new `Pending` approval gate (Cluster 350). JSON columns are stored as
/// TEXT in SQLite.
pub async fn create(pool: &SqlitePool, gate: &NewApprovalGate) -> Result<ApprovalGate, StoreError> {
    let id = ApprovalGateId::new();
    let schema_text = gate
        .schema
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_approval_gates
             (id, workspace_id, thread_id, requested_by, prompt, schema, state, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(id.0)
    .bind(gate.workspace_id.0)
    .bind(gate.thread_id.map(|t| t.0))
    .bind(gate.requested_by.0)
    .bind(&gate.prompt)
    .bind(schema_text)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_gate(&row)
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
         SET state = ?, content = ?, resolved_by = ?, resolved_at = ?
         WHERE id = ? AND state = 'pending'
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(state.as_str())
    .bind(content_text)
    .bind(resolved_by.0)
    .bind(&now)
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
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        resolved_at: row.get::<Option<DateTime<Utc>>, _>("resolved_at"),
    })
}
