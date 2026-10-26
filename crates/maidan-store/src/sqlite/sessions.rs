use chrono::{DateTime, Utc};
use maidan_types::{
    ApiTokenId, MaidanSession, MemberId, NewMaidanSession, OidcIdentityId, SessionId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const SESSION_COLUMNS: &str =
    "id, workspace_id, member_id, api_token_id, oidc_identity_id, created_at, expires_at";

/// A session belongs to a live member: not tombstoned (SCIM delete) and not
/// SCIM-deactivated. The insert checks this in its own statement, so a sign-in
/// that resolved its member before a SCIM delete or deactivation committed
/// cannot mint a session after it. Binds: member id, workspace id, member id.
const MEMBER_CAN_SIGN_IN: &str = "EXISTS (SELECT 1 FROM maidan_members m
              WHERE m.id = ? AND m.workspace_id = ? AND m.tombstoned_at IS NULL)
         AND NOT EXISTS (SELECT 1 FROM maidan_scim_users u
              WHERE u.member_id = ? AND NOT u.active)";

fn member_cannot_sign_in() -> StoreError {
    StoreError::Conflict("member is deleted or deactivated".into())
}

pub async fn create(pool: &SqlitePool, new: NewMaidanSession) -> Result<MaidanSession, StoreError> {
    let mut conn = pool.acquire().await?;
    create_on(&mut conn, new).await
}

pub(crate) async fn create_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewMaidanSession,
) -> Result<MaidanSession, StoreError> {
    // A session id is a credential (the cookie carries it), so it stays v4.
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_sessions
            (id, workspace_id, member_id, api_token_id, oidc_identity_id, created_at, expires_at)
         SELECT ?, ?, ?, ?, ?, ?, ?
         WHERE {MEMBER_CAN_SIGN_IN}
         RETURNING {SESSION_COLUMNS}"
    ))
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(new.api_token_id.map(|t| t.0))
    .bind(new.oidc_identity_id.map(|i| i.0))
    .bind(now)
    .bind(new.expires_at)
    .bind(new.member_id.0)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .fetch_optional(conn)
    .await?
    .ok_or_else(member_cannot_sign_in)?;
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
    super::audit::append_scoped(&mut tx, session.workspace_id, audit(&session)).await?;
    tx.commit().await?;
    Ok(session)
}

pub async fn get(pool: &SqlitePool, id: SessionId) -> Result<MaidanSession, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(&format!(
        "SELECT {SESSION_COLUMNS}
         FROM maidan_sessions s
         WHERE s.id = ? AND s.expires_at > ?
           AND EXISTS (SELECT 1 FROM maidan_members m
                WHERE m.id = s.member_id AND m.tombstoned_at IS NULL)
           AND NOT EXISTS (SELECT 1 FROM maidan_scim_users u
                WHERE u.member_id = s.member_id AND NOT u.active)"
    ))
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
    let row = sqlx::query(&format!(
        "DELETE FROM maidan_sessions WHERE id = ?
         RETURNING {SESSION_COLUMNS}"
    ))
    .bind(id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let session = row_to_session(&row)?;
    super::audit::append_scoped(&mut tx, session.workspace_id, audit(&session)).await?;
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
        api_token_id: row.get::<Option<Uuid>, _>("api_token_id").map(ApiTokenId),
        oidc_identity_id: row
            .get::<Option<Uuid>, _>("oidc_identity_id")
            .map(OidcIdentityId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
    })
}
