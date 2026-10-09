use chrono::{DateTime, Utc};
use maidan_types::{
    IdentityWorkspace, MemberId, NewOidcIdentity, NewOidcPendingAuth, OidcIdentity, OidcIdentityId,
    OidcPendingAuth, OidcPendingTarget, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn upsert_identity(
    pool: &SqlitePool,
    new: NewOidcIdentity,
) -> Result<OidcIdentity, StoreError> {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_oidc_identities
            (id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, issuer, subject)
         DO UPDATE SET
            member_id = excluded.member_id,
            email = excluded.email,
            last_login_at = excluded.last_login_at
         RETURNING id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.issuer)
    .bind(&new.subject)
    .bind(new.member_id.0)
    .bind(new.email.as_deref())
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    row_to_identity(&row)
}

pub async fn get_identity(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    issuer: &str,
    subject: &str,
) -> Result<OidcIdentity, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at
         FROM maidan_oidc_identities
         WHERE workspace_id = ? AND issuer = ? AND subject = ?",
    )
    .bind(workspace_id.0)
    .bind(issuer)
    .bind(subject)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_identity(&row)
}

/// The workspaces the identity `identity_id` can switch to: every workspace
/// where the same issuer and subject have an identity row, with the member each
/// maps to, newest sign-in first, at most `limit`. The identity's own workspace
/// is always listed. Another is left out when its member is SCIM-deactivated or
/// frozen. An unknown id lists nothing.
pub async fn list_identity_workspaces(
    pool: &SqlitePool,
    identity_id: OidcIdentityId,
    limit: i64,
) -> Result<Vec<IdentityWorkspace>, StoreError> {
    let rows = sqlx::query(
        "SELECT o.workspace_id, w.name, o.member_id, m.handle, o.last_login_at
         FROM maidan_oidc_identities me
         JOIN maidan_oidc_identities o
           ON o.issuer = me.issuer AND o.subject = me.subject
         JOIN maidan_workspaces w ON w.id = o.workspace_id
         JOIN maidan_members m ON m.id = o.member_id AND m.workspace_id = o.workspace_id
         WHERE me.id = ?
           AND (o.id = me.id OR (
                NOT EXISTS (SELECT 1 FROM maidan_scim_users s
                            WHERE s.member_id = o.member_id AND s.active = 0)
            AND NOT EXISTS (SELECT 1 FROM maidan_member_freezes f
                            WHERE f.member_id = o.member_id)))
         ORDER BY o.last_login_at DESC, o.workspace_id
         LIMIT ?",
    )
    .bind(identity_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| IdentityWorkspace {
            workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
            workspace_name: row.get::<String, _>("name"),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            handle: row.get::<String, _>("handle"),
            last_login_at: row.get::<DateTime<Utc>, _>("last_login_at"),
        })
        .collect())
}

/// The workspaces the front door may sign `(issuer, subject)` in to: every
/// workspace where that issuer and subject have an identity row, with the
/// member each maps to, newest sign-in first, at most `limit`. A workspace
/// whose member is SCIM-deactivated or frozen is left out, so the front door
/// never lands anyone where they could not sign in. Nothing else is matched:
/// not an email, not a handle.
pub async fn list_subject_workspaces(
    pool: &SqlitePool,
    issuer: &str,
    subject: &str,
    limit: i64,
) -> Result<Vec<IdentityWorkspace>, StoreError> {
    let rows = sqlx::query(
        "SELECT o.workspace_id, w.name, o.member_id, m.handle, o.last_login_at
         FROM maidan_oidc_identities o
         JOIN maidan_workspaces w ON w.id = o.workspace_id
         JOIN maidan_members m ON m.id = o.member_id AND m.workspace_id = o.workspace_id
         WHERE o.issuer = ? AND o.subject = ?
           AND NOT EXISTS (SELECT 1 FROM maidan_scim_users s
                           WHERE s.member_id = o.member_id AND s.active = 0)
           AND NOT EXISTS (SELECT 1 FROM maidan_member_freezes f
                           WHERE f.member_id = o.member_id)
         ORDER BY o.last_login_at DESC, o.workspace_id
         LIMIT ?",
    )
    .bind(issuer)
    .bind(subject)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| IdentityWorkspace {
            workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
            workspace_name: row.get::<String, _>("name"),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            handle: row.get::<String, _>("handle"),
            last_login_at: row.get::<DateTime<Utc>, _>("last_login_at"),
        })
        .collect())
}

pub async fn insert_pending(pool: &SqlitePool, new: NewOidcPendingAuth) -> Result<(), StoreError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_oidc_pending
            (state, kind, workspace_id, nonce, pkce_verifier, return_to, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&new.state)
    .bind(new.target.kind())
    .bind(new.target.workspace_id().map(|w| w.0))
    .bind(&new.nonce)
    .bind(&new.pkce_verifier)
    .bind(new.return_to.as_deref())
    .bind(now)
    .bind(new.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn take_pending(pool: &SqlitePool, state: &str) -> Result<OidcPendingAuth, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(
        "DELETE FROM maidan_oidc_pending
         WHERE state = ? AND expires_at > ?
         RETURNING state, kind, workspace_id, nonce, pkce_verifier, return_to, expires_at",
    )
    .bind(state)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(OidcPendingAuth {
        state: row.get("state"),
        target: pending_target(row.get("kind"), row.get::<Option<Uuid>, _>("workspace_id"))?,
        nonce: row.get("nonce"),
        pkce_verifier: row.get("pkce_verifier"),
        return_to: row.get("return_to"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
    })
}

/// A pending row's kind and workspace, as one target. The CHECK on the table
/// allows only these two shapes; anything else is refused rather than guessed.
fn pending_target(
    kind: String,
    workspace_id: Option<Uuid>,
) -> Result<OidcPendingTarget, StoreError> {
    match (kind.as_str(), workspace_id) {
        ("sign_in", Some(id)) => Ok(OidcPendingTarget::Workspace(WorkspaceId(id))),
        ("front_door", None) => Ok(OidcPendingTarget::FrontDoor),
        _ => Err(StoreError::InvalidInput(format!(
            "pending sign-in has kind {kind:?} and workspace {workspace_id:?}"
        ))),
    }
}

fn row_to_identity(row: &sqlx::sqlite::SqliteRow) -> Result<OidcIdentity, StoreError> {
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
