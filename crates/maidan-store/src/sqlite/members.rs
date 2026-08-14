use chrono::{DateTime, Utc};
use maidan_types::{Event, Member, MemberId, MemberKind, NewMember, StoredEvent, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

pub async fn create(pool: &SqlitePool, new: NewMember) -> Result<Member, StoreError> {
    let id = Uuid::new_v4();
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
    .fetch_one(pool)
    .await
    .map_err(map_member_err)?;
    row_to_member(&row)
}

/// Insert a member and append its `MemberJoined` event in one transaction
/// (Cluster 213 transactional outbox).
pub async fn create_with_event(
    pool: &SqlitePool,
    new: NewMember,
) -> Result<(Member, StoredEvent), StoreError> {
    let id = Uuid::new_v4();
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

pub async fn get_by_handle(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    handle: &str,
) -> Result<Member, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at
         FROM maidan_members
         WHERE workspace_id = ? AND handle = ? AND tombstoned_at IS NULL",
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
