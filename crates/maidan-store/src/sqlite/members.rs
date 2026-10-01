use chrono::{DateTime, Utc};
use maidan_types::{Event, Member, MemberId, MemberKind, NewMember, StoredEvent, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

pub async fn create(pool: &SqlitePool, new: NewMember) -> Result<Member, StoreError> {
    let mut conn = pool.acquire().await?;
    create_on(&mut conn, new).await
}

pub(crate) async fn create_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewMember,
) -> Result<Member, StoreError> {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_members (id, workspace_id, handle, display_name, kind, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.handle)
    .bind(new.display_name.as_deref())
    .bind(new.kind.as_str())
    .bind(now)
    .bind(now)
    .fetch_one(&mut *conn)
    .await
    .map_err(map_member_err)?;
    row_to_member(&row)
}

/// Insert a member and append its `MemberJoined` event in one transaction.
pub async fn create_with_event(
    pool: &SqlitePool,
    new: NewMember,
) -> Result<(Member, StoredEvent), StoreError> {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let workspace_id = new.workspace_id;
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_members (id, workspace_id, handle, display_name, kind, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.handle)
    .bind(new.display_name.as_deref())
    .bind(new.kind.as_str())
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_member_err)?;
    let member = row_to_member(&row)?;
    let event = Event::MemberJoined {
        occurred_at: Utc::now(),
        workspace_id,
        member: member.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((member, stored))
}

/// `NotFound` unless `member_id` names a member of `workspace_id`. A request
/// naming a member is refused the same way whether the id names no member or
/// another workspace's: left to the foreign key, the first answered 500 and
/// the second was accepted.
pub(crate) async fn ensure_in_workspace(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query("SELECT 1 FROM maidan_members WHERE id = ? AND workspace_id = ?")
        .bind(member_id.0)
        .bind(workspace_id.0)
        .fetch_optional(conn)
        .await?
        .map(|_| ())
        .ok_or(StoreError::NotFound)
}

pub async fn get(pool: &SqlitePool, id: MemberId) -> Result<Member, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at
         FROM maidan_members WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_member(&row)
}

/// The workspace's member with this handle, ignoring case. `Alice` and
/// `alice` are the same handle. A tombstoned member is `NotFound`.
pub async fn get_by_handle(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    handle: &str,
) -> Result<Member, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at
         FROM maidan_members
         WHERE workspace_id = ? AND lower(handle) = lower(?) AND tombstoned_at IS NULL",
    )
    .bind(workspace_id.0)
    .bind(handle)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_member(&row)
}

pub async fn list(pool: &SqlitePool, workspace_id: WorkspaceId) -> Result<Vec<Member>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at
         FROM maidan_members WHERE workspace_id = ? ORDER BY handle ASC",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_member).collect()
}

/// Change a member's handle. `false` when the workspace has no such member; a
/// handle another member of the workspace holds is a [`StoreError::Conflict`].
pub(crate) async fn rename_on(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    handle: &str,
) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE maidan_members SET handle = ?, updated_at = ? WHERE id = ? AND workspace_id = ?",
    )
    .bind(handle)
    .bind(Utc::now())
    .bind(member_id.0)
    .bind(workspace_id.0)
    .execute(&mut *conn)
    .await
    .map_err(map_member_err)?;
    Ok(done.rows_affected() > 0)
}

fn map_member_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict("handle already exists in workspace".into());
        }
    }
    StoreError::Database(err)
}

fn row_to_member(row: &sqlx::sqlite::SqliteRow) -> Result<Member, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = match kind_str.as_str() {
        "human" => MemberKind::Human,
        "agent" => MemberKind::Agent,
        other => {
            return Err(StoreError::InvalidInput(format!(
                "unknown member kind: {other}"
            )));
        }
    };
    Ok(Member {
        id: MemberId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        handle: row.get("handle"),
        display_name: row.get("display_name"),
        kind,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}
