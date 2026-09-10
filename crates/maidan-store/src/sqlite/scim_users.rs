//! SCIM 2.0 provisioning-link queries (Cluster 366, SCIM-as-OIDC-P3): the
//! `maidan_scim_users` table mapping a SCIM User to a Maidan member.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, ScimUser, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_scim(row: &sqlx::sqlite::SqliteRow) -> ScimUser {
    ScimUser {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        external_id: row.get::<Option<String>, _>("external_id"),
        active: row.get::<bool, _>("active"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

const COLS: &str = "member_id, workspace_id, external_id, active, created_at, updated_at";

pub async fn create(
    pool: &SqlitePool,
    member_id: MemberId,
    workspace_id: WorkspaceId,
    external_id: Option<&str>,
    active: bool,
) -> Result<ScimUser, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_scim_users
             (member_id, workspace_id, external_id, active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING {COLS}"
    ))
    .bind(member_id.0)
    .bind(workspace_id.0)
    .bind(external_id)
    .bind(active)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_scim(&row))
}

pub async fn get(pool: &SqlitePool, member_id: MemberId) -> Result<Option<ScimUser>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_scim_users WHERE member_id = ?"
    ))
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_scim))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<ScimUser>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_scim_users WHERE workspace_id = ?
         ORDER BY created_at ASC, member_id ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_scim).collect())
}

pub async fn update(
    pool: &SqlitePool,
    member_id: MemberId,
    external_id: Option<&str>,
    active: bool,
) -> Result<Option<ScimUser>, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "UPDATE maidan_scim_users
         SET external_id = ?, active = ?, updated_at = ?
         WHERE member_id = ? RETURNING {COLS}"
    ))
    .bind(external_id)
    .bind(active)
    .bind(&now)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_scim))
}

pub async fn delete(pool: &SqlitePool, member_id: MemberId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_scim_users WHERE member_id = ?")
        .bind(member_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
