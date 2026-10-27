use chrono::{DateTime, Utc};
use maidan_types::{
    Event, IdentityWorkspace, MaidanSession, MemberId, MemberKind, NewMaidanSession, NewMember,
    NewOidcIdentity, NewOidcPendingAuth, NewOidcSignup, NewWaitlistEntry, OidcIdentity,
    OidcIdentityId, OidcPendingAuth, OidcPendingTarget, OidcSignupCore, OidcSignupOutcome,
    WaitlistEntry, WaitlistEntryId, WaitlistStatus, Workspace, WorkspaceId,
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
         -- The first row wins: a later call never moves an identity to
         -- another member, so two racing first logins agree (#1642).
         DO UPDATE SET
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
            (state, kind, workspace_id, nonce, pkce_verifier, return_to, invite_entry_id, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&new.state)
    .bind(new.target.kind())
    .bind(new.target.workspace_id().map(|w| w.0))
    .bind(&new.nonce)
    .bind(&new.pkce_verifier)
    .bind(new.return_to.as_deref())
    .bind(new.invite_entry_id.map(|id| id.0))
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
         RETURNING state, kind, workspace_id, nonce, pkce_verifier, return_to, invite_entry_id, expires_at",
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
        invite_entry_id: row
            .get::<Option<Uuid>, _>("invite_entry_id")
            .map(WaitlistEntryId),
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
        ("sign_up", None) => Ok(OidcPendingTarget::SignUp),
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

const WAITLIST_COLUMNS: &str = "id, email, use_case, consented_at, status, invite_token_hash, \
    invite_expires_at, workspace_id, created_at, updated_at";

fn row_to_waitlist_entry(row: &sqlx::sqlite::SqliteRow) -> Result<WaitlistEntry, StoreError> {
    let status: String = row.get("status");
    Ok(WaitlistEntry {
        id: WaitlistEntryId(row.get::<Uuid, _>("id")),
        email: row.get("email"),
        use_case: row.get("use_case"),
        consented_at: row.get::<DateTime<Utc>, _>("consented_at"),
        status: WaitlistStatus::parse(&status).ok_or_else(|| {
            StoreError::InvalidInput(format!("waitlist entry has status {status:?}"))
        })?,
        invite_token_hash: row.get("invite_token_hash"),
        invite_expires_at: row.get::<Option<DateTime<Utc>>, _>("invite_expires_at"),
        workspace_id: row.get::<Option<Uuid>, _>("workspace_id").map(WorkspaceId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}

pub async fn insert_waitlist_entry(
    pool: &SqlitePool,
    new: NewWaitlistEntry,
) -> Result<WaitlistEntry, StoreError> {
    let id = Uuid::now_v7();
    let now = Utc::now();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_waitlist_entries
            (id, email, email_lower, use_case, consented_at, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'waiting', ?, ?)
         RETURNING {WAITLIST_COLUMNS}"
    ))
    .bind(id)
    .bind(&new.email)
    .bind(new.email.to_lowercase())
    .bind(&new.use_case)
    .bind(new.consented_at)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            StoreError::Conflict(format!("a waitlist entry already exists for {}", new.email))
        }
        other => StoreError::from(other),
    })?;
    row_to_waitlist_entry(&row)
}

pub async fn list_waitlist_entries(pool: &SqlitePool) -> Result<Vec<WaitlistEntry>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {WAITLIST_COLUMNS} FROM maidan_waitlist_entries ORDER BY created_at, id"
    ))
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_waitlist_entry).collect()
}

pub async fn get_waitlist_entry(
    pool: &SqlitePool,
    id: WaitlistEntryId,
) -> Result<WaitlistEntry, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {WAITLIST_COLUMNS} FROM maidan_waitlist_entries WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_waitlist_entry(&row)
}

pub async fn get_waitlist_entry_by_invite_hash(
    pool: &SqlitePool,
    token_hash: &str,
) -> Result<WaitlistEntry, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {WAITLIST_COLUMNS} FROM maidan_waitlist_entries WHERE invite_token_hash = ?"
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_waitlist_entry(&row)
}

pub async fn set_waitlist_invite(
    pool: &SqlitePool,
    id: WaitlistEntryId,
    token_hash: String,
    expires_at: DateTime<Utc>,
) -> Result<WaitlistEntry, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(&format!(
        "UPDATE maidan_waitlist_entries
         SET status = 'invited', invite_token_hash = ?, invite_expires_at = ?, updated_at = ?
         WHERE id = ? AND status != 'signed_up'
         RETURNING {WAITLIST_COLUMNS}"
    ))
    .bind(&token_hash)
    .bind(expires_at)
    .bind(now)
    .bind(id.0)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(row) => row_to_waitlist_entry(&row),
        None => {
            // Either no such entry, or it already signed up and an invite
            // must not be minted over a completed sign-up.
            let exists = sqlx::query("SELECT 1 AS one FROM maidan_waitlist_entries WHERE id = ?")
                .bind(id.0)
                .fetch_optional(pool)
                .await?;
            match exists {
                Some(_) => Err(StoreError::Conflict(
                    "the waitlist entry has already signed up".into(),
                )),
                None => Err(StoreError::NotFound),
            }
        }
    }
}

/// The sign-up transaction: workspace, admin member, identity and session
/// in one commit, with both audit rows and the waitlist bookkeeping. The
/// invite gate is checked against the row inside the transaction and the
/// consuming update must still match, so two callbacks racing one invite
/// cannot both land.
pub async fn complete_signup(
    pool: &SqlitePool,
    new: NewOidcSignup,
    signup_audit: crate::AuditFor<OidcSignupCore>,
    session_audit: crate::AuditFor<MaidanSession>,
) -> Result<OidcSignupOutcome, StoreError> {
    let now = Utc::now();
    let email_lower = new.email.to_lowercase();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    // `BEGIN IMMEDIATE` (above) holds the write lock from the first
    // statement, so two callbacks for one identity serialize and the second
    // sees the first's identity row.
    // The handler checks this before charging the domain budget; checked
    // again here under the lock, with the front door's own filters, so a
    // concurrent callback for the same identity cannot mint a second workspace.
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM maidan_oidc_identities o
             JOIN maidan_members m ON m.id = o.member_id AND m.workspace_id = o.workspace_id
             WHERE o.issuer = ? AND o.subject = ?
               AND NOT EXISTS (SELECT 1 FROM maidan_scim_users s
                               WHERE s.member_id = o.member_id AND s.active = 0)
               AND NOT EXISTS (SELECT 1 FROM maidan_member_freezes f
                               WHERE f.member_id = o.member_id))",
    )
    .bind(&new.issuer)
    .bind(&new.subject)
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Err(StoreError::Conflict(crate::SIGNUP_IDENTITY_TAKEN.into()));
    }

    if let Some(entry_id) = new.invite_entry_id {
        let gate = sqlx::query(
            "SELECT status, invite_expires_at, email_lower
             FROM maidan_waitlist_entries WHERE id = ?",
        )
        .bind(entry_id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        let status: String = gate.get("status");
        let expires_at: Option<DateTime<Utc>> = gate.get("invite_expires_at");
        let entry_email: String = gate.get("email_lower");
        if status != "invited" || expires_at.is_none_or(|e| e <= now) || entry_email != email_lower
        {
            return Err(StoreError::Conflict("the invite is no longer valid".into()));
        }
    }

    let workspace_id = Uuid::now_v7();
    let row = sqlx::query(
        "INSERT INTO maidan_workspaces (id, name, created_at, updated_at, origin)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id, name, created_at, updated_at, tombstoned_at",
    )
    .bind(workspace_id)
    .bind(&new.workspace_name)
    .bind(now)
    .bind(now)
    .bind(if new.open { "open" } else { "signup" })
    .fetch_one(&mut *tx)
    .await?;
    let workspace = Workspace {
        id: WorkspaceId(row.get::<Uuid, _>("id")),
        name: row.get("name"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    };
    super::events::append_in_tx(
        &mut tx,
        &Event::WorkspaceCreated {
            occurred_at: now,
            workspace: workspace.clone(),
        },
    )
    .await?;

    let member = super::members::create_on(
        &mut tx,
        NewMember {
            workspace_id: workspace.id,
            handle: new.handle.clone(),
            display_name: None,
            kind: MemberKind::Human,
        },
    )
    .await?;
    super::events::append_in_tx(
        &mut tx,
        &Event::MemberJoined {
            occurred_at: now,
            workspace_id: workspace.id,
            member: member.clone(),
        },
    )
    .await?;

    let identity_row = sqlx::query(
        "INSERT INTO maidan_oidc_identities
            (id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id, workspace_id, issuer, subject, member_id, email, created_at, last_login_at",
    )
    .bind(Uuid::now_v7())
    .bind(workspace.id.0)
    .bind(&new.issuer)
    .bind(&new.subject)
    .bind(member.id.0)
    .bind(&new.email)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let identity = row_to_identity(&identity_row)?;

    let session = super::sessions::create_on(
        &mut tx,
        NewMaidanSession {
            workspace_id: workspace.id,
            member_id: member.id,
            api_token_id: None,
            oidc_identity_id: Some(identity.id),
            expires_at: new.session_expires_at,
        },
    )
    .await?;

    // Waitlist bookkeeping in the same commit: consume the invite, or mark
    // a waiting entry with this email as signed up by the domain path.
    match new.invite_entry_id {
        Some(entry_id) => {
            let updated = sqlx::query(
                "UPDATE maidan_waitlist_entries
                 SET status = 'signed_up', invite_token_hash = NULL, invite_expires_at = NULL,
                     workspace_id = ?, updated_at = ?
                 WHERE id = ? AND status = 'invited'",
            )
            .bind(workspace.id.0)
            .bind(now)
            .bind(entry_id.0)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Conflict("the invite is no longer valid".into()));
            }
        }
        None => {
            sqlx::query(
                "UPDATE maidan_waitlist_entries
                 SET status = 'signed_up', workspace_id = ?, updated_at = ?
                 WHERE email_lower = ? AND status != 'signed_up'",
            )
            .bind(workspace.id.0)
            .bind(now)
            .bind(&email_lower)
            .execute(&mut *tx)
            .await?;
        }
    }

    let core = OidcSignupCore {
        workspace: workspace.clone(),
        member: member.clone(),
        identity: identity.clone(),
    };
    super::audit::append_scoped(&mut tx, core.workspace.id, signup_audit(&core)).await?;
    super::audit::append_scoped(&mut tx, session.workspace_id, session_audit(&session)).await?;
    tx.commit().await?;
    Ok(OidcSignupOutcome {
        workspace,
        member,
        identity,
        session,
    })
}
