use chrono::{DateTime, Utc};
use maidan_types::{MaidanSession, MemberId, NewMaidanSession, SessionId, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewMaidanSession) -> Result<MaidanSession, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_sessions
            (id, workspace_id, member_id, csrf_secret, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, workspace_id, member_id, csrf_secret, created_at, expires_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(&new.csrf_secret)
    .bind(new.expires_at)
    .fetch_one(pool)
    .await?;
    row_to_session(&row)
}

pub async fn get(pool: &PgPool, id: SessionId) -> Result<MaidanSession, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, member_id, csrf_secret, created_at, expires_at
         FROM maidan_sessions
         WHERE id = $1 AND expires_at > NOW()",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_session(&row)
}

pub async fn delete(pool: &PgPool, id: SessionId) -> Result<(), StoreError> {
    let result = sqlx::query("DELETE FROM maidan_sessions WHERE id = $1")
        .bind(id.0)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

fn row_to_session(row: &sqlx::postgres::PgRow) -> Result<MaidanSession, StoreError> {
    Ok(MaidanSession {
        id: SessionId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        csrf_secret: row.get("csrf_secret"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
    })
}
