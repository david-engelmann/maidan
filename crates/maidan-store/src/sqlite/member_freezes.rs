//! Member-freeze kill-switch store (Cluster 372, SQLite twin of pg 0077).
//! Freezing records the freeze AND drops the member's active leases in one tx.

use chrono::{DateTime, Utc};
use maidan_types::{MemberFreeze, MemberId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "member_id, frozen_at, frozen_by, reason";

fn row_to_freeze(row: &sqlx::sqlite::SqliteRow) -> MemberFreeze {
    MemberFreeze {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        frozen_at: row.get::<DateTime<Utc>, _>("frozen_at"),
        frozen_by: MemberId(row.get::<Uuid, _>("frozen_by")),
        reason: row.get::<Option<String>, _>("reason"),
    }
}

pub async fn freeze(
    pool: &SqlitePool,
    member_id: MemberId,
    frozen_by: MemberId,
    reason: Option<&str>,
) -> Result<(MemberFreeze, u64), StoreError> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
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
         SET assignee_id = NULL, assignment_expires_at = NULL, claim_lease_id = NULL,
             work_started_at = NULL, updated_at = ?
         WHERE assignee_id = ? AND tombstoned_at IS NULL AND state NOT IN ('closed', 'archived')",
    )
    .bind(&now)
    .bind(member_id.0)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((freeze, released.rows_affected()))
}

pub async fn unfreeze(pool: &SqlitePool, member_id: MemberId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_member_freezes WHERE member_id = ?")
        .bind(member_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
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
