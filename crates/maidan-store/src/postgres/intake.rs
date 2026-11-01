//! A workspace's intake queue: incoming work proposals that are not tasks
//! yet. Proposing appends `IntakeProposed` with the row; triage moves a
//! `proposed` item to `accepted` or `rejected` once, with its audit row and
//! its event, in one transaction.

use chrono::{DateTime, Utc};
use maidan_types::{
    Event, IntakeDecision, IntakeItem, IntakeSource, IntakeState, MemberId, NewAuditEvent,
    NewIntakeItem, StoredEvent, ThreadId, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{audit, events};
use crate::audit_scope::AuditOwner;
use crate::error::StoreError;

const COLS: &str = "id, workspace_id, title, body, source, proposed_by, state, thread_id, \
                    triaged_by, triaged_at, reject_reason, created_at";

fn row_to_item(row: &sqlx::postgres::PgRow) -> Result<IntakeItem, StoreError> {
    let source: String = row.get("source");
    let state: String = row.get("state");
    Ok(IntakeItem {
        id: row.get::<Uuid, _>("id"),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        title: row.get("title"),
        body: row.get("body"),
        source: IntakeSource::parse(&source)
            .ok_or_else(|| StoreError::InvalidInput(format!("intake source {source}")))?,
        proposed_by: MemberId(row.get::<Uuid, _>("proposed_by")),
        state: IntakeState::parse(&state)
            .ok_or_else(|| StoreError::InvalidInput(format!("intake state {state}")))?,
        thread_id: row.get::<Option<Uuid>, _>("thread_id").map(ThreadId),
        triaged_by: row.get::<Option<Uuid>, _>("triaged_by").map(MemberId),
        triaged_at: row.get::<Option<DateTime<Utc>>, _>("triaged_at"),
        reject_reason: row.get("reject_reason"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

/// Record a proposal and its `IntakeProposed` event together.
pub async fn create(
    pool: &PgPool,
    new: &NewIntakeItem,
) -> Result<(IntakeItem, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_intake_items
             (id, workspace_id, title, body, source, proposed_by, state, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'proposed', $7)
         RETURNING {COLS}"
    ))
    .bind(Uuid::now_v7())
    .bind(new.workspace_id.0)
    .bind(&new.title)
    .bind(&new.body)
    .bind(new.source.as_str())
    .bind(new.proposed_by.0)
    .bind(Utc::now())
    .fetch_one(&mut *tx)
    .await?;
    let item = row_to_item(&row)?;
    let stored = events::append_in_tx(
        &mut tx,
        &Event::IntakeProposed {
            occurred_at: item.created_at,
            workspace_id: item.workspace_id,
            intake_id: item.id,
            title: item.title.clone(),
            source: item.source,
            proposed_by: item.proposed_by,
        },
    )
    .await?;
    tx.commit().await?;
    Ok((item, stored))
}

pub async fn get(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    id: Uuid,
) -> Result<Option<IntakeItem>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_intake_items WHERE workspace_id = $1 AND id = $2"
    ))
    .bind(workspace_id.0)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_item).transpose()
}

/// Newest first. `cursor` is the prior page's last id (exclusive).
pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    state: Option<IntakeState>,
    cursor: Option<Uuid>,
    limit: i64,
) -> Result<Vec<IntakeItem>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_intake_items
         WHERE workspace_id = $1
           AND ($2::text IS NULL OR state = $3)
           AND ($4::uuid IS NULL OR id < $5)
         ORDER BY id DESC LIMIT $6"
    ))
    .bind(workspace_id.0)
    .bind(state.map(IntakeState::as_str))
    .bind(state.map(IntakeState::as_str))
    .bind(cursor)
    .bind(cursor)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_item).collect()
}

/// Triage a proposed item once: the state change, its audit row and its
/// event commit together. A missing item is `NotFound`; one already
/// triaged is a `Conflict`.
pub async fn triage(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    id: Uuid,
    triaged_by: MemberId,
    decision: &IntakeDecision,
    event: NewAuditEvent,
) -> Result<(IntakeItem, StoredEvent), StoreError> {
    let (state, thread_id, reason) = match decision {
        IntakeDecision::Accept { thread_id } => (IntakeState::Accepted, Some(thread_id.0), None),
        IntakeDecision::Reject { reason } => (IntakeState::Rejected, None, Some(reason.as_str())),
    };
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "UPDATE maidan_intake_items
            SET state = $1, thread_id = $2, reject_reason = $3, triaged_by = $4, triaged_at = $5
          WHERE workspace_id = $6 AND id = $7 AND state = 'proposed'
          RETURNING {COLS}"
    ))
    .bind(state.as_str())
    .bind(thread_id)
    .bind(reason)
    .bind(triaged_by.0)
    .bind(now)
    .bind(workspace_id.0)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        let exists =
            sqlx::query("SELECT 1 FROM maidan_intake_items WHERE workspace_id = $1 AND id = $2")
                .bind(workspace_id.0)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        return Err(match exists {
            Some(_) => StoreError::Conflict("intake item already triaged".into()),
            None => StoreError::NotFound,
        });
    };
    let item = row_to_item(&row)?;
    let change = match decision {
        IntakeDecision::Accept { thread_id } => Event::IntakeAccepted {
            occurred_at: now,
            workspace_id,
            intake_id: id,
            thread_id: *thread_id,
            triaged_by,
        },
        IntakeDecision::Reject { reason } => Event::IntakeRejected {
            occurred_at: now,
            workspace_id,
            intake_id: id,
            reason: reason.clone(),
            triaged_by,
        },
    };
    let stored = events::append_in_tx(&mut tx, &change).await?;
    audit::append_scoped(&mut tx, AuditOwner::Workspace(workspace_id), event).await?;
    tx.commit().await?;
    Ok((item, stored))
}
