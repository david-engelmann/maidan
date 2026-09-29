//! Member-freeze kill-switch store. Freezing records the freeze, drops the
//! member's active leases and appends `MemberFrozen` in one tx; an unfreeze
//! that lifts a freeze appends `MemberUnfrozen` with it.

use chrono::{DateTime, Utc};
use maidan_types::{Event, MemberFreeze, MemberId, StoredEvent, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

const COLS: &str = "member_id, frozen_at, frozen_by, reason";

fn row_to_freeze(row: &sqlx::sqlite::SqliteRow) -> MemberFreeze {
    MemberFreeze {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        frozen_at: row.get::<DateTime<Utc>, _>("frozen_at"),
        frozen_by: MemberId(row.get::<Uuid, _>("frozen_by")),
        reason: row.get::<Option<String>, _>("reason"),
    }
}

/// The member's workspace, read in the change's transaction for its event.
async fn member_workspace_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    member_id: MemberId,
) -> Result<WorkspaceId, StoreError> {
    let row = sqlx::query("SELECT workspace_id FROM maidan_members WHERE id = ?")
        .bind(member_id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok(WorkspaceId(row.get::<Uuid, _>("workspace_id")))
}

pub async fn freeze(
    pool: &SqlitePool,
    member_id: MemberId,
    frozen_by: MemberId,
    reason: Option<&str>,
) -> Result<(MemberFreeze, u64, StoredEvent), StoreError> {
    let mut conn = pool.acquire().await?;
    freeze_on(&mut conn, member_id, frozen_by, reason).await
}

pub(crate) async fn freeze_on(
    conn: &mut sqlx::SqliteConnection,
    member_id: MemberId,
    frozen_by: MemberId,
    reason: Option<&str>,
) -> Result<(MemberFreeze, u64, StoredEvent), StoreError> {
    let now = Utc::now().to_rfc3339();
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let workspace_id = member_workspace_in_tx(&mut tx, member_id).await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_member_freezes (member_id, frozen_at, frozen_by, reason)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (member_id) DO UPDATE SET
             frozen_at = excluded.frozen_at, frozen_by = excluded.frozen_by, reason = excluded.reason
         RETURNING {COLS}"
    ))
    .bind(member_id.0)
    .bind(&now)
    .bind(frozen_by.0)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    let freeze = row_to_freeze(&row);
    let released = sqlx::query(
        "UPDATE maidan_threads
         SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL, claimed_at = NULL,
             work_started_at = NULL, updated_at = ?
         WHERE assignee_id = ? AND tombstoned_at IS NULL AND state NOT IN ('closed', 'archived')",
    )
    .bind(&now)
    .bind(member_id.0)
    .execute(&mut *tx)
    .await?
    .rows_affected();
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

/// Postgres twin: `None` when the member was not frozen, and nothing is
/// appended.
pub async fn unfreeze(
    pool: &SqlitePool,
    member_id: MemberId,
    unfrozen_by: MemberId,
) -> Result<Option<StoredEvent>, StoreError> {
    let mut conn = pool.acquire().await?;
    unfreeze_on(&mut conn, member_id, unfrozen_by).await
}

pub(crate) async fn unfreeze_on(
    conn: &mut sqlx::SqliteConnection,
    member_id: MemberId,
    unfrozen_by: MemberId,
) -> Result<Option<StoredEvent>, StoreError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let done = sqlx::query("DELETE FROM maidan_member_freezes WHERE member_id = ?")
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

pub async fn is_frozen(pool: &SqlitePool, member_id: MemberId) -> Result<bool, StoreError> {
    let row = sqlx::query("SELECT 1 FROM maidan_member_freezes WHERE member_id = ?")
        .bind(member_id.0)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub async fn get(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Option<MemberFreeze>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_member_freezes WHERE member_id = ?"
    ))
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_freeze))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<MemberFreeze>, StoreError> {
    let rows = sqlx::query(
        "SELECT f.member_id, f.frozen_at, f.frozen_by, f.reason
         FROM maidan_member_freezes f
         JOIN maidan_members m ON m.id = f.member_id
         WHERE m.workspace_id = ?
         ORDER BY f.frozen_at DESC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_freeze).collect())
}
