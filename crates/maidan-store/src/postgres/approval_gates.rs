use chrono::{DateTime, Utc};
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, Event, MemberId, NewApprovalGate, StoredEvent,
    ThreadId, WorkspaceId,
};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::StoreError;

const GATE_COLUMNS: &str = "id, workspace_id, thread_id, requested_by, prompt, schema, state, \
     content, resolved_by, requested_actor_id, resolved_actor_id, created_at, resolved_at";

/// Open a new `Pending` approval gate. See the SQLite twin. `schema` binds
/// directly to the JSONB column.
pub async fn create(pool: &PgPool, gate: &NewApprovalGate) -> Result<ApprovalGate, StoreError> {
    let mut tx = pool.begin().await?;
    let gate = create_in_tx(&mut tx, gate).await?;
    tx.commit().await?;
    Ok(gate)
}

async fn create_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    gate: &NewApprovalGate,
) -> Result<ApprovalGate, StoreError> {
    let id = ApprovalGateId::new();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_approval_gates
             (id, workspace_id, thread_id, requested_by, prompt, schema, state,
              requested_actor_id)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7)
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(id.0)
    .bind(gate.workspace_id.0)
    .bind(gate.thread_id.map(|t| t.0))
    .bind(gate.requested_by.0)
    .bind(&gate.prompt)
    .bind(gate.schema.as_ref())
    .bind(crate::attribution::delegate_acting_for(gate.requested_by).map(|m| m.0))
    .fetch_one(&mut **tx)
    .await?;
    Ok(row_to_gate(&row))
}

pub async fn create_with_event(
    pool: &PgPool,
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

pub async fn get(pool: &PgPool, id: ApprovalGateId) -> Result<Option<ApprovalGate>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {GATE_COLUMNS} FROM maidan_approval_gates WHERE id = $1"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_gate))
}

/// The pending gates in a workspace, oldest first — the queryable held-gate list.
pub async fn list_pending(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<ApprovalGate>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {GATE_COLUMNS} FROM maidan_approval_gates
         WHERE workspace_id = $1 AND state = 'pending'
         ORDER BY created_at ASC
         LIMIT $2"
    ))
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_gate).collect())
}

/// Resolve a `Pending` gate to accept/decline/cancel. Compare-and-set on
/// `pending` (the `WHERE ... state = 'pending'`) so a second resolver — or a
/// late answer after a cancel — is a no-op: returns the resolved gate, or `None`
/// if it was already resolved or the id is unknown. `state` must be a resolved
/// variant; passing `Pending` is a caller bug.
pub async fn resolve(
    pool: &PgPool,
    id: ApprovalGateId,
    resolved_by: MemberId,
    state: ApprovalGateState,
    content: Option<&serde_json::Value>,
) -> Result<Option<ApprovalGate>, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_approval_gates
         SET state = $2, content = $3, resolved_by = $4, resolved_at = now(),
             resolved_actor_id = $5
         WHERE id = $1 AND state = 'pending'
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(id.0)
    .bind(state.as_str())
    .bind(content)
    .bind(resolved_by.0)
    .bind(crate::attribution::delegate_acting_for(resolved_by).map(|m| m.0))
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_gate))
}

fn row_to_gate(row: &sqlx::postgres::PgRow) -> ApprovalGate {
    ApprovalGate {
        id: ApprovalGateId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        thread_id: row.get::<Option<Uuid>, _>("thread_id").map(ThreadId),
        requested_by: MemberId(row.get::<Uuid, _>("requested_by")),
        prompt: row.get::<String, _>("prompt"),
        schema: row.get::<Option<serde_json::Value>, _>("schema"),
        state: ApprovalGateState::parse(&row.get::<String, _>("state"))
            .unwrap_or(ApprovalGateState::Pending),
        content: row.get::<Option<serde_json::Value>, _>("content"),
        resolved_by: row.get::<Option<Uuid>, _>("resolved_by").map(MemberId),
        requested_actor_id: row
            .get::<Option<Uuid>, _>("requested_actor_id")
            .map(MemberId),
        resolved_actor_id: row
            .get::<Option<Uuid>, _>("resolved_actor_id")
            .map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        resolved_at: row.get::<Option<DateTime<Utc>>, _>("resolved_at"),
    }
}
