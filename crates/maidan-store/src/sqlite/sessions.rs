use chrono::{DateTime, Utc};
use maidan_types::{MaidanSession, MemberId, NewMaidanSession, SessionId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewMaidanSession) -> Result<MaidanSession, StoreError> {
    let mut conn = pool.acquire().await?;
    create_on(&mut conn, new).await
}

async fn create_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewMaidanSession,
) -> Result<MaidanSession, StoreError> {
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
    .fetch_one(conn)
    .await?;
    row_to_session(&row)
}

/// [`create`], with its audit row in the same transaction (D-A).
pub async fn create_audited(
    pool: &SqlitePool,
    new: NewMaidanSession,
    audit: crate::AuditFor<MaidanSession>,
) -> Result<MaidanSession, StoreError> {
    let mut tx = pool.begin().await?;
    let session = create_on(&mut tx, new).await?;
    super::audit::append_counted(&mut tx, audit(&session)).await?;
    tx.commit().await?;
    Ok(session)
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

/// Delete a session and write `audit` for it in the same transaction (D-A).
/// `NotFound`, with nothing written, if it is already gone.
pub async fn delete_audited(
    pool: &SqlitePool,
    id: SessionId,
    audit: crate::AuditFor<MaidanSession>,
) -> Result<MaidanSession, StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "DELETE FROM maidan_sessions WHERE id = ?
         RETURNING id, workspace_id, member_id, csrf_secret, created_at, expires_at",
    )
    .bind(id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let session = row_to_session(&row)?;
    super::audit::append_counted(&mut tx, audit(&session)).await?;
    tx.commit().await?;
    Ok(session)
}

/// Delete a session only if it has expired. It already grants nothing, so its
/// removal changes no one's authority and is not audited.
pub async fn delete_expired(pool: &SqlitePool, id: SessionId) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_sessions WHERE id = ? AND expires_at <= ?")
        .bind(id.0)
        .bind(Utc::now())
        .execute(pool)
        .await?;
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
