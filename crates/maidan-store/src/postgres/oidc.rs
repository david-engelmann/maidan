use chrono::{DateTime, Utc};
use maidan_types::{
    MemberId, NewOidcIdentity, NewOidcPendingAuth, OidcIdentity, OidcIdentityId, OidcPendingAuth,
    WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn upsert_identity(
    pool: &PgPool,
    new: NewOidcIdentity,
) -> Result<OidcIdentity, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_oidc_identities
            (id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
         ON CONFLICT (workspace_id, issuer, subject)
         DO UPDATE SET
            member_id = EXCLUDED.member_id,
            email = EXCLUDED.email,
            last_login_at = NOW()
         RETURNING id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.issuer)
    .bind(&new.subject)
    .bind(new.member_id.0)
    .bind(new.email.as_deref())
    .fetch_one(pool)
    .await?;
    row_to_identity(&row)
}

pub async fn get_identity(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    issuer: &str,
    subject: &str,
) -> Result<OidcIdentity, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at
         FROM maidan_oidc_identities
         WHERE workspace_id = $1 AND issuer = $2 AND subject = $3",
    )
    .bind(workspace_id.0)
    .bind(issuer)
    .bind(subject)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_identity(&row)
}

pub async fn insert_pending(pool: &PgPool, new: NewOidcPendingAuth) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_oidc_pending
            (state, workspace_id, nonce, pkce_verifier, return_to, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&new.state)
    .bind(new.workspace_id.0)
    .bind(&new.nonce)
    .bind(&new.pkce_verifier)
    .bind(new.return_to.as_deref())
    .bind(new.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn take_pending(pool: &PgPool, state: &str) -> Result<OidcPendingAuth, StoreError> {
    let row = sqlx::query(
        "DELETE FROM maidan_oidc_pending
         WHERE state = $1 AND expires_at > NOW()
         RETURNING state, workspace_id, nonce, pkce_verifier, return_to, expires_at",
    )
    .bind(state)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(OidcPendingAuth {
        state: row.get("state"),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        nonce: row.get("nonce"),
        pkce_verifier: row.get("pkce_verifier"),
        return_to: row.get("return_to"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
    })
}

fn row_to_identity(row: &sqlx::postgres::PgRow) -> Result<OidcIdentity, StoreError> {
    Ok(OidcIdentity {
        id: OidcIdentityId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        issuer: row.get("issuer"),
        subject: row.get("subject"),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        email: row.get("email"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        last_login_at: row.get::<DateTime<Utc>, _>("last_login_at"),
    })
}
