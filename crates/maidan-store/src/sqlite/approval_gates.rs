use chrono::{DateTime, SecondsFormat, Utc};
use maidan_types::{
    ApprovalConfirmation, ApprovalGate, ApprovalGateId, ApprovalGateState, ApprovalPolicy,
    ApprovalRisk, ConfirmOutcome, Event, GateDecisionVia, MemberId, NewApprovalConfirmation,
    NewApprovalGate, StoredEvent, ThreadId, WorkspaceId, DM_CHANNEL_NAME,
};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::a2a::PendingGateQuery;
use crate::error::StoreError;
use crate::thread_access::readable_row;

const GATE_COLUMNS: &str = "id, workspace_id, thread_id, requested_by, prompt, schema, state, \
     content, resolved_by, requested_actor_id, resolved_actor_id, created_at, resolved_at, risk, \
     decided_via_client, decided_via_client_version, model_asked, decided_via_source, \
     decided_via_client_id";

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
              requested_actor_id, risk)
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)
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
    .bind(gate.risk.as_str())
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
    let mut conn = pool.acquire().await?;
    resolve_on(&mut conn, id, resolved_by, state, content, None).await
}

async fn resolve_on(
    conn: &mut sqlx::SqliteConnection,
    id: ApprovalGateId,
    resolved_by: MemberId,
    state: ApprovalGateState,
    content: Option<&serde_json::Value>,
    via: Option<&GateDecisionVia>,
) -> Result<Option<ApprovalGate>, StoreError> {
    let content_text = content.map(serde_json::to_string).transpose()?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "UPDATE maidan_approval_gates
         SET state = ?, content = ?, resolved_by = ?, resolved_at = ?, resolved_actor_id = ?,
             decided_via_client = ?, decided_via_client_version = ?, model_asked = ?,
             decided_via_source = ?, decided_via_client_id = ?
         WHERE id = ? AND state = 'pending'
         RETURNING {GATE_COLUMNS}"
    ))
    .bind(state.as_str())
    .bind(content_text)
    .bind(resolved_by.0)
    .bind(&now)
    .bind(crate::attribution::delegate_acting_for(resolved_by).map(|m| m.0))
    .bind(via.and_then(|v| v.client_name.as_deref()))
    .bind(via.and_then(|v| v.client_version.as_deref()))
    .bind(via.is_some_and(|v| v.model_asked))
    .bind(via.map(|v| v.client_source.as_str()))
    .bind(via.and_then(|v| v.client_id.as_deref()))
    .bind(id.0)
    .fetch_optional(&mut *conn)
    .await?;
    row.as_ref().map(row_to_gate).transpose()
}

/// See the Postgres twin.
pub async fn resolve_audited(
    pool: &SqlitePool,
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

/// See the Postgres twin.
pub async fn get_policy(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<ApprovalPolicy, StoreError> {
    let mut conn = pool.acquire().await?;
    get_policy_on(&mut conn, workspace_id).await
}

async fn get_policy_on(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
) -> Result<ApprovalPolicy, StoreError> {
    let stored: Option<(String, i64)> = sqlx::query_as(
        "SELECT confirm_at, CAST(confirm_link_ttl_seconds AS BIGINT)
         FROM maidan_approval_policies WHERE workspace_id = ?",
    )
    .bind(workspace_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(crate::approval_policy::policy(workspace_id, stored))
}

/// See the Postgres twin.
pub async fn set_policy_audited(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    confirm_at: Option<ApprovalRisk>,
    confirm_link_ttl_seconds: Option<u32>,
    audit: crate::AuditFor<ApprovalPolicy>,
) -> Result<ApprovalPolicy, StoreError> {
    let mut tx = pool.begin().await?;
    match (confirm_at, confirm_link_ttl_seconds) {
        (None, None) => {
            sqlx::query("DELETE FROM maidan_approval_policies WHERE workspace_id = ?")
                .bind(workspace_id.0)
                .execute(&mut *tx)
                .await?;
        }
        (at, ttl) => {
            let at = at.unwrap_or(crate::approval_policy::DEFAULT_CONFIRM_AT);
            let ttl = ttl.unwrap_or(crate::approval_policy::DEFAULT_CONFIRM_LINK_TTL_SECONDS);
            sqlx::query(
                "INSERT INTO maidan_approval_policies
                     (workspace_id, confirm_at, confirm_link_ttl_seconds, updated_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT (workspace_id) DO UPDATE
                 SET confirm_at = excluded.confirm_at,
                     confirm_link_ttl_seconds = excluded.confirm_link_ttl_seconds,
                     updated_at = excluded.updated_at",
            )
            .bind(workspace_id.0)
            .bind(at.as_str())
            .bind(i64::from(ttl))
            .bind(ts(Utc::now()))
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

/// See the Postgres twin. Times are millisecond `...Z` text, so the expiry
/// comparison in the upsert is a string comparison that orders correctly.
pub async fn issue_confirmation(
    pool: &SqlitePool,
    new: &NewApprovalConfirmation,
    audit: crate::AuditFor<ApprovalConfirmation>,
) -> Result<(ApprovalConfirmation, bool), StoreError> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(&format!(
        "INSERT INTO maidan_approval_confirmations
             (gate_id, member_id, workspace_id, actor_id, nonce, token_hash, client_name,
              client_version, client_id, client_source, note, created_at, expires_at, used_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
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
    .bind(ts(new.now))
    .bind(ts(new.expires_at))
    .fetch_optional(&mut *tx)
    .await?;
    let (confirmation, fresh) = match inserted {
        Some(row) => (row_to_confirmation(&row)?, true),
        None => {
            let row = sqlx::query(&format!(
                "SELECT {CONFIRMATION_COLUMNS} FROM maidan_approval_confirmations
                 WHERE gate_id = ? AND member_id = ?"
            ))
            .bind(new.gate_id.0)
            .bind(new.member_id.0)
            .fetch_one(&mut *tx)
            .await?;
            (row_to_confirmation(&row)?, false)
        }
    };
    if fresh {
        super::audit::append_counted(&mut tx, audit(&confirmation)).await?;
    }
    tx.commit().await?;
    Ok((confirmation, fresh))
}

pub async fn get_confirmation_by_token(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<Option<ApprovalConfirmation>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {CONFIRMATION_COLUMNS} FROM maidan_approval_confirmations WHERE token_hash = ?"
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_confirmation).transpose()
}

/// See the Postgres twin.
pub async fn list_live_confirmations(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    now: DateTime<Utc>,
) -> Result<Vec<ApprovalConfirmation>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {CONFIRMATION_COLUMNS} FROM maidan_approval_confirmations
         WHERE workspace_id = ? AND used_at IS NULL AND expires_at > ?
         ORDER BY created_at ASC
         LIMIT 500"
    ))
    .bind(workspace_id.0)
    .bind(ts(now))
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_confirmation).collect()
}

/// See the Postgres twin. SQLite's transaction holds the database's write
/// lock from the first write, and the spend is the first write, so a second
/// confirm waits and then finds the row used.
pub async fn confirm(
    pool: &SqlitePool,
    token_hash: &str,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    now: DateTime<Utc>,
    audit: crate::AuditFor<ApprovalGate>,
) -> Result<ConfirmOutcome, StoreError> {
    let mut tx = pool.begin().await?;
    // Spend first, conditionally: the UPDATE is the lock, so the later read
    // and resolve see this transaction's view and no other confirm's.
    let spent = sqlx::query(&format!(
        "UPDATE maidan_approval_confirmations SET used_at = ?
         WHERE token_hash = ? AND workspace_id = ? AND member_id = ?
           AND used_at IS NULL AND expires_at > ?
         RETURNING {CONFIRMATION_COLUMNS}"
    ))
    .bind(ts(now))
    .bind(token_hash)
    .bind(workspace_id.0)
    .bind(member_id.0)
    .bind(ts(now))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(confirmation) = spent.as_ref().map(row_to_confirmation).transpose()? else {
        return Ok(ConfirmOutcome::NotFound);
    };
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

fn row_to_confirmation(row: &sqlx::sqlite::SqliteRow) -> Result<ApprovalConfirmation, StoreError> {
    Ok(ApprovalConfirmation {
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
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
        used_at: row.get::<Option<DateTime<Utc>>, _>("used_at"),
    })
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
        risk: ApprovalRisk::parse(&row.get::<String, _>("risk")).unwrap_or_default(),
        decided_via: crate::approval_policy::decided_via(
            row.get("decided_via_client"),
            row.get("decided_via_client_version"),
            row.get("decided_via_client_id"),
            row.get("decided_via_source"),
            row.get::<bool, _>("model_asked"),
        ),
    })
}
