//! Member-freeze kill-switch store (Cluster 372, Wave 2 #20): the
//! `maidan_member_freezes` table. Freezing a member records the freeze AND drops
//! their active leases (releases their claimed threads) in one transaction;
//! `claim_next` refuses a frozen member (enforced in `threads.rs`, Cluster 372.2).
//! See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{MemberFreeze, MemberId, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "member_id, frozen_at, frozen_by, reason";

fn row_to_freeze(row: &sqlx::postgres::PgRow) -> MemberFreeze {
    MemberFreeze {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        frozen_at: row.get::<DateTime<Utc>, _>("frozen_at"),
        frozen_by: MemberId(row.get::<Uuid, _>("frozen_by")),
        reason: row.get::<Option<String>, _>("reason"),
    }
}

/// Freeze a member: upsert the freeze row and release every active claim they
/// hold (drop leases), in one transaction. Returns the freeze + the number of
/// threads released. Re-freezing refreshes `frozen_by`/`reason`/`frozen_at`.
pub async fn freeze(
    pool: &PgPool,
    member_id: MemberId,
    frozen_by: MemberId,
    reason: Option<&str>,
) -> Result<(MemberFreeze, u64), StoreError> {
    let mut tx = pool.begin().await?;
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
    // Drop leases: release the member's active (non-terminal, non-tombstoned)
    // claims so the work returns to the queue for another agent.
    let released = sqlx::query(
        "UPDATE maidan_threads
         SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL,
             work_started_at = NULL, updated_at = NOW()
         WHERE assignee_id = $1 AND tombstoned_at IS NULL AND state NOT IN ('closed', 'archived')",
    )
    .bind(member_id.0)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((freeze, released.rows_affected()))
}

pub async fn unfreeze(pool: &PgPool, member_id: MemberId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_member_freezes WHERE member_id = $1")
        .bind(member_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
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
