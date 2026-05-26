use chrono::{DateTime, Utc};
use maidan_types::{MaidanSession, MemberId, NewMaidanSession, SessionId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewMaidanSession) -> Result<MaidanSession, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_sessions
            (id, workspace_id, member_id, csrf_secret, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, member_id, csrf_secret, created_at, expires_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(&new.csrf_secret)
    .bind(now)
    .bind(new.expires_at)
    .fetch_one(pool)
    .await?;
    row_to_session(&row)
}

pub async fn get(pool: &SqlitePool, id: SessionId) -> Result<MaidanSession, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(
        "SELECT id, workspace_id, member_id, csrf_secret, created_at, expires_at
         FROM maidan_sessions
         WHERE id = ? AND expires_at > ?",
    )
    .bind(id.0)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_session(&row)
}

pub async fn delete(pool: &SqlitePool, id: SessionId) -> Result<(), StoreError> {
    let result = sqlx::query("DELETE FROM maidan_sessions WHERE id = ?")
        .bind(id.0)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

fn row_to_session(row: &sqlx::sqlite::SqliteRow) -> Result<MaidanSession, StoreError> {
    Ok(MaidanSession {
        id: SessionId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        csrf_secret: row.get("csrf_secret"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
    })
}
