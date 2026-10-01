//! Member-freeze kill-switch store: the `maidan_member_freezes` table. Freezing
//! a member records the freeze, drops their active leases (releases their
//! claimed threads) and appends `MemberFrozen` in one transaction; an unfreeze
//! that lifts a freeze appends `MemberUnfrozen` with it. `claim_next` refuses a
//! frozen member (enforced in `threads.rs`). See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{Event, MemberFreeze, MemberId, StoredEvent, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

const COLS: &str = "member_id, frozen_at, frozen_by, reason";

fn row_to_freeze(row: &sqlx::postgres::PgRow) -> MemberFreeze {
    MemberFreeze {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        frozen_at: row.get::<DateTime<Utc>, _>("frozen_at"),
        frozen_by: MemberId(row.get::<Uuid, _>("frozen_by")),
        reason: row.get::<Option<String>, _>("reason"),
    }
}

/// The member's workspace, read in the change's transaction for its event.
async fn member_workspace_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    member_id: MemberId,
) -> Result<WorkspaceId, StoreError> {
    let row = sqlx::query("SELECT workspace_id FROM maidan_members WHERE id = $1")
        .bind(member_id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok(WorkspaceId(row.get::<Uuid, _>("workspace_id")))
}

/// Freeze a member: upsert the freeze row, release every active claim they
/// hold (drop leases) and append `MemberFrozen`, in one transaction. Returns
/// the freeze, the number of threads released and the event. Re-freezing
/// refreshes `frozen_by`/`reason`/`frozen_at`.
pub async fn freeze(
    pool: &PgPool,
    member_id: MemberId,
    frozen_by: MemberId,
    reason: Option<&str>,
) -> Result<(MemberFreeze, u64, StoredEvent), StoreError> {
    let mut conn = pool.acquire().await?;
    freeze_on(&mut conn, member_id, frozen_by, reason).await
}

pub(crate) async fn freeze_on(
    conn: &mut sqlx::PgConnection,
    member_id: MemberId,
    frozen_by: MemberId,
    reason: Option<&str>,
) -> Result<(MemberFreeze, u64, StoredEvent), StoreError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let workspace_id = member_workspace_in_tx(&mut tx, member_id).await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_member_freezes (member_id, frozen_at, frozen_by, reason)
         VALUES ($1, NOW(), $2, $3)
         ON CONFLICT (member_id) DO UPDATE SET
             frozen_at = NOW(), frozen_by = excluded.frozen_by, reason = excluded.reason
         RETURNING {COLS}"
    ))
    .bind(member_id.0)
    .bind(frozen_by.0)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    let freeze = row_to_freeze(&row);
    // Drop leases: release the member's active claims, charging each
    // acknowledged claim's worked time in this same transaction.
    let released = super::threads::release_member_claims_in_tx(&mut tx, member_id).await?;
    let stored = events::append_in_tx(
        &mut tx,
        &Event::MemberFrozen {
            occurred_at: freeze.frozen_at,
            workspace_id,
            member_id,
            frozen_by,
            reason: freeze.reason.clone(),
            released: i64::try_from(released).unwrap_or(i64::MAX),
        },
    )
    .await?;
    tx.commit().await?;
    Ok((freeze, released, stored))
}

/// Lift a freeze and append `MemberUnfrozen`, together. `None` when the member
/// was not frozen: nothing changed, so nothing is appended.
pub async fn unfreeze(
    pool: &PgPool,
    member_id: MemberId,
    unfrozen_by: MemberId,
) -> Result<Option<StoredEvent>, StoreError> {
    let mut conn = pool.acquire().await?;
    unfreeze_on(&mut conn, member_id, unfrozen_by).await
}

pub(crate) async fn unfreeze_on(
    conn: &mut sqlx::PgConnection,
    member_id: MemberId,
    unfrozen_by: MemberId,
) -> Result<Option<StoredEvent>, StoreError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let done = sqlx::query("DELETE FROM maidan_member_freezes WHERE member_id = $1")
        .bind(member_id.0)
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        tx.commit().await?;
        return Ok(None);
    }
    let workspace_id = member_workspace_in_tx(&mut tx, member_id).await?;
    let stored = events::append_in_tx(
        &mut tx,
        &Event::MemberUnfrozen {
            occurred_at: Utc::now(),
            workspace_id,
            member_id,
            unfrozen_by,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Some(stored))
}

pub async fn is_frozen(pool: &PgPool, member_id: MemberId) -> Result<bool, StoreError> {
    let row = sqlx::query("SELECT 1 FROM maidan_member_freezes WHERE member_id = $1")
        .bind(member_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub async fn get(pool: &PgPool, member_id: MemberId) -> Result<Option<MemberFreeze>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_member_freezes WHERE member_id = $1"
    ))
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_freeze))
}

/// The frozen members in a workspace (joined through `maidan_members`).
pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<MemberFreeze>, StoreError> {
    let rows = sqlx::query(
        "SELECT f.member_id, f.frozen_at, f.frozen_by, f.reason
         FROM maidan_member_freezes f
         JOIN maidan_members m ON m.id = f.member_id
         WHERE m.workspace_id = $1
         ORDER BY f.frozen_at DESC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_freeze).collect())
}
