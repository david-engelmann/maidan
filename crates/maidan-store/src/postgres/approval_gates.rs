use chrono::{DateTime, Utc};
use maidan_types::{
    ApprovalConfirmation, ApprovalGate, ApprovalGateId, ApprovalGateState, ApprovalPolicy,
    ApprovalRisk, ConfirmOutcome, Event, GateDecisionVia, MemberId, NewApprovalConfirmation,
    NewApprovalGate, StoredEvent, ThreadId, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::a2a::PendingGateQuery;
use crate::error::StoreError;
use crate::thread_access::readable_row;

const GATE_COLUMNS: &str = "id, workspace_id, thread_id, requested_by, prompt, schema, state, \
     content, resolved_by, requested_actor_id, resolved_actor_id, created_at, resolved_at, risk, \
     decided_via_client, decided_via_client_version, model_asked, decided_via_source, \
     decided_via_client_id";

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
              requested_actor_id, risk)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8)
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(id.0)
    .bind(gate.workspace_id.0)
    .bind(gate.thread_id.map(|t| t.0))
    .bind(gate.requested_by.0)
    .bind(&gate.prompt)
    .bind(gate.schema.as_ref())
    .bind(crate::attribution::delegate_acting_for(gate.requested_by).map(|m| m.0))
    .bind(gate.risk.as_str())
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

/// The filters `page_pending` and `count_pending` share: `$1` workspace,
/// `$2` thread, `$3` created since, `$4` reader, `$5` the DM channel name.
fn pending_filters() -> String {
    let readable = readable_row("maidan_approval_gates.thread_id", "$1", "$4::uuid", "$5");
    format!(
        "workspace_id = $1 AND state = 'pending'
           AND ($2::uuid IS NULL OR thread_id = $2)
           AND ($3::timestamptz IS NULL OR created_at >= $3)
           AND {readable}"
    )
}

/// A keyset page of the pending gates, newest first. See [`PendingGateQuery`].
pub async fn page_pending(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    query: PendingGateQuery,
) -> Result<Vec<ApprovalGate>, StoreError> {
    let (before_at, before_id) = match query.before {
        Some((at, id)) => (Some(at), id.map(|id| id.0)),
        None => (None, None),
    };
    let rows = sqlx::query(&format!(
        "SELECT {GATE_COLUMNS} FROM maidan_approval_gates
         WHERE {}
           AND ($6::timestamptz IS NULL OR created_at < $6
                OR (created_at = $6 AND $7::uuid IS NOT NULL AND id < $7))
         ORDER BY created_at DESC, id DESC
         LIMIT $8",
        pending_filters()
    ))
    .bind(workspace_id.0)
    .bind(query.thread_id.map(|t| t.0))
    .bind(query.created_since)
    .bind(query.readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .bind(before_at)
    .bind(before_id)
    .bind(query.limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_gate).collect())
}

/// How many pending gates match `query`'s filters.
pub async fn count_pending(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    query: PendingGateQuery,
) -> Result<i64, StoreError> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM maidan_approval_gates WHERE {}",
        pending_filters()
    ))
    .bind(workspace_id.0)
    .bind(query.thread_id.map(|t| t.0))
    .bind(query.created_since)
    .bind(query.readable_by.map(|m| m.0))
    .bind(DM_CHANNEL_NAME)
    .fetch_one(pool)
    .await?)
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
    let mut conn = pool.acquire().await?;
    resolve_on(&mut conn, id, resolved_by, state, content, None).await
}

async fn resolve_on(
    conn: &mut sqlx::PgConnection,
    id: ApprovalGateId,
    resolved_by: MemberId,
    state: ApprovalGateState,
    content: Option<&serde_json::Value>,
    via: Option<&GateDecisionVia>,
) -> Result<Option<ApprovalGate>, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_approval_gates
         SET state = $2, content = $3, resolved_by = $4, resolved_at = now(),
             resolved_actor_id = $5, decided_via_client = $6,
             decided_via_client_version = $7, model_asked = $8,
             decided_via_source = $9, decided_via_client_id = $10
         WHERE id = $1 AND state = 'pending'
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(id.0)
    .bind(state.as_str())
    .bind(content)
    .bind(resolved_by.0)
    .bind(crate::attribution::delegate_acting_for(resolved_by).map(|m| m.0))
    .bind(via.and_then(|v| v.client_name.as_deref()))
    .bind(via.and_then(|v| v.client_version.as_deref()))
    .bind(via.is_some_and(|v| v.model_asked))
    .bind(via.map(|v| v.client_source.as_str()))
    .bind(via.and_then(|v| v.client_id.as_deref()))
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.as_ref().map(row_to_gate))
}

/// [`resolve`] for a decision a model made, recording the client it came
/// through, with its audit row in the same transaction. The row is written
/// only when the gate actually resolved.
pub async fn resolve_audited(
    pool: &PgPool,
    id: ApprovalGateId,
    resolved_by: MemberId,
    state: ApprovalGateState,
    content: Option<&serde_json::Value>,
    via: &GateDecisionVia,
    audit: crate::AuditFor<ApprovalGate>,
) -> Result<Option<ApprovalGate>, StoreError> {
    let mut tx = pool.begin().await?;
    let gate = resolve_on(&mut tx, id, resolved_by, state, content, Some(via)).await?;
    if let Some(gate) = &gate {
        super::audit::append_counted(&mut tx, audit(gate)).await?;
    }
    tx.commit().await?;
    Ok(gate)
}

/// The workspace's confirmation threshold; `low` when it has set none.
pub async fn get_policy(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<ApprovalPolicy, StoreError> {
    let mut conn = pool.acquire().await?;
    get_policy_on(&mut conn, workspace_id).await
}

async fn get_policy_on(
    conn: &mut sqlx::PgConnection,
    workspace_id: WorkspaceId,
) -> Result<ApprovalPolicy, StoreError> {
    let at: Option<String> = sqlx::query_scalar(
        "SELECT confirm_at FROM maidan_approval_policies WHERE workspace_id = $1",
    )
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(crate::approval_policy::policy(workspace_id, at.as_deref()))
}

/// Set the threshold, or return to the default with `None`, with its audit
/// row in the same transaction.
pub async fn set_policy_audited(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    confirm_at: Option<ApprovalRisk>,
    audit: crate::AuditFor<ApprovalPolicy>,
) -> Result<ApprovalPolicy, StoreError> {
    let mut tx = pool.begin().await?;
    match confirm_at {
        Some(at) => {
            sqlx::query(
                "INSERT INTO maidan_approval_policies (workspace_id, confirm_at, updated_at)
                 VALUES ($1, $2, now())
                 ON CONFLICT (workspace_id) DO UPDATE
                 SET confirm_at = excluded.confirm_at, updated_at = excluded.updated_at",
            )
            .bind(workspace_id.0)
            .bind(at.as_str())
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM maidan_approval_policies WHERE workspace_id = $1")
                .bind(workspace_id.0)
                .execute(&mut *tx)
                .await?;
        }
    }
    let policy = get_policy_on(&mut tx, workspace_id).await?;
    super::audit::append_counted(&mut tx, audit(&policy)).await?;
    tx.commit().await?;
    Ok(policy)
}

const CONFIRMATION_COLUMNS: &str = "gate_id, member_id, workspace_id, actor_id, nonce, \
     client_name, client_version, client_id, client_source, note, created_at, expires_at, used_at";

/// Issue a confirmation for a gate and member, or hand back the live one.
/// The upsert replaces a row only once it is used or expired, so while one
/// is live every repeat call gets it back (`false`) and nothing new is minted.
/// `audit` is written only for a new one.
pub async fn issue_confirmation(
    pool: &PgPool,
    new: &NewApprovalConfirmation,
    audit: crate::AuditFor<ApprovalConfirmation>,
) -> Result<(ApprovalConfirmation, bool), StoreError> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(&format!(
        "INSERT INTO maidan_approval_confirmations
             (gate_id, member_id, workspace_id, actor_id, nonce, token_hash, client_name,
              client_version, client_id, client_source, note, created_at, expires_at, used_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, NULL)
         ON CONFLICT (gate_id, member_id) DO UPDATE
         SET workspace_id = excluded.workspace_id, actor_id = excluded.actor_id,
             nonce = excluded.nonce, token_hash = excluded.token_hash,
             client_name = excluded.client_name, client_version = excluded.client_version,
             client_id = excluded.client_id, client_source = excluded.client_source,
             note = excluded.note, created_at = excluded.created_at,
             expires_at = excluded.expires_at, used_at = NULL
         WHERE maidan_approval_confirmations.used_at IS NOT NULL
            OR maidan_approval_confirmations.expires_at <= excluded.created_at
         RETURNING {CONFIRMATION_COLUMNS}"
    ))
    .bind(new.gate_id.0)
    .bind(new.member_id.0)
    .bind(new.workspace_id.0)
    .bind(new.actor_id.map(|m| m.0))
    .bind(new.nonce)
    .bind(&new.token_hash)
    .bind(new.client_name.as_deref())
    .bind(new.client_version.as_deref())
    .bind(new.client_id.as_deref())
    .bind(new.client_source.as_str())
    .bind(new.note.as_deref())
    .bind(new.now)
    .bind(new.expires_at)
    .fetch_optional(&mut *tx)
    .await?;
    let (confirmation, fresh) = match inserted {
        Some(row) => (row_to_confirmation(&row), true),
        None => {
            let row = sqlx::query(&format!(
                "SELECT {CONFIRMATION_COLUMNS} FROM maidan_approval_confirmations
                 WHERE gate_id = $1 AND member_id = $2"
            ))
            .bind(new.gate_id.0)
            .bind(new.member_id.0)
            .fetch_one(&mut *tx)
            .await?;
            (row_to_confirmation(&row), false)
        }
    };
    if fresh {
        super::audit::append_counted(&mut tx, audit(&confirmation)).await?;
    }
    tx.commit().await?;
    Ok((confirmation, fresh))
}

pub async fn get_confirmation_by_token(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<ApprovalConfirmation>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {CONFIRMATION_COLUMNS} FROM maidan_approval_confirmations WHERE token_hash = $1"
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_confirmation))
}

/// The workspace's unused, unexpired confirmations, oldest first.
pub async fn list_live_confirmations(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    now: DateTime<Utc>,
) -> Result<Vec<ApprovalConfirmation>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {CONFIRMATION_COLUMNS} FROM maidan_approval_confirmations
         WHERE workspace_id = $1 AND used_at IS NULL AND expires_at > $2
         ORDER BY created_at ASC
         LIMIT 500"
    ))
    .bind(workspace_id.0)
    .bind(now)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_confirmation).collect())
}

/// Spend a live confirmation and accept its gate, in one transaction. The
/// row is locked first, so two confirms of one link cannot both accept.
pub async fn confirm(
    pool: &PgPool,
    token_hash: &str,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    now: DateTime<Utc>,
    audit: crate::AuditFor<ApprovalGate>,
) -> Result<ConfirmOutcome, StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "SELECT {CONFIRMATION_COLUMNS} FROM maidan_approval_confirmations
         WHERE token_hash = $1 FOR UPDATE"
    ))
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(confirmation) = row.as_ref().map(row_to_confirmation) else {
        return Ok(ConfirmOutcome::NotFound);
    };
    if confirmation.workspace_id != workspace_id
        || confirmation.member_id != member_id
        || !confirmation.is_live(now)
    {
        return Ok(ConfirmOutcome::NotFound);
    }
    sqlx::query(
        "UPDATE maidan_approval_confirmations SET used_at = $3
         WHERE gate_id = $1 AND member_id = $2",
    )
    .bind(confirmation.gate_id.0)
    .bind(confirmation.member_id.0)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let via = crate::approval_policy::via(&confirmation);
    let content = crate::approval_policy::note_content(confirmation.note.as_deref());
    let gate = resolve_on(
        &mut tx,
        confirmation.gate_id,
        member_id,
        ApprovalGateState::Accepted,
        content.as_ref(),
        Some(&via),
    )
    .await?;
    let outcome = match gate {
        Some(gate) => {
            super::audit::append_counted(&mut tx, audit(&gate)).await?;
            ConfirmOutcome::Accepted(Box::new(gate))
        }
        None => ConfirmOutcome::GateResolved,
    };
    tx.commit().await?;
    Ok(outcome)
}

fn row_to_confirmation(row: &sqlx::postgres::PgRow) -> ApprovalConfirmation {
    ApprovalConfirmation {
        gate_id: ApprovalGateId(row.get::<Uuid, _>("gate_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        actor_id: row.get::<Option<Uuid>, _>("actor_id").map(MemberId),
        nonce: row.get::<Uuid, _>("nonce"),
        client_name: row.get("client_name"),
        client_version: row.get("client_version"),
        client_id: row.get("client_id"),
        client_source: crate::approval_policy::source(row.get("client_source")),
        note: row.get("note"),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        used_at: row.get("used_at"),
    }
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
        risk: ApprovalRisk::parse(&row.get::<String, _>("risk")).unwrap_or_default(),
        decided_via: crate::approval_policy::decided_via(
            row.get("decided_via_client"),
            row.get("decided_via_client_version"),
            row.get("decided_via_client_id"),
            row.get("decided_via_source"),
            row.get::<bool, _>("model_asked"),
        ),
    }
}
