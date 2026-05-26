use chrono::{DateTime, Utc};
use maidan_types::{Member, MemberId, MemberKind, NewMember, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewMember) -> Result<Member, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_members (id, workspace_id, handle, display_name, kind)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.handle)
    .bind(new.display_name.as_deref())
    .bind(new.kind.as_str())
    .fetch_one(pool)
    .await
    .map_err(map_member_err)?;
    row_to_member(&row)
}

pub async fn get(pool: &PgPool, id: MemberId) -> Result<Member, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at
         FROM maidan_members WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_member(&row)
}

pub async fn get_by_handle(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    handle: &str,
) -> Result<Member, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at
         FROM maidan_members
         WHERE workspace_id = $1 AND handle = $2 AND tombstoned_at IS NULL",
    )
    .bind(workspace_id.0)
    .bind(handle)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_member(&row)
}

pub async fn list(pool: &PgPool, workspace_id: WorkspaceId) -> Result<Vec<Member>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at
         FROM maidan_members WHERE workspace_id = $1 ORDER BY handle ASC",
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

fn row_to_member(row: &sqlx::postgres::PgRow) -> Result<Member, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = match kind_str.as_str() {
        "human" => MemberKind::Human,
        "agent" => MemberKind::Agent,
        other => {
            return Err(StoreError::InvalidInput(format!(
                "unknown member kind: {other}"
            )))
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
