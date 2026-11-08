use chrono::{DateTime, Utc};
use maidan_types::{
    AcceptWorkspaceInvite, Event, MaidanSession, MemberId, MemberKind, NewMaidanSession, NewMember,
    NewWorkspaceInvite, WorkspaceId, WorkspaceInvite, WorkspaceInviteAcceptance, WorkspaceInviteId,
    WorkspaceInviteRole,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::workspace_invites::{
    nth_handle, validate_new, HANDLE_ATTEMPTS, INVITE_ALREADY_MEMBER, INVITE_NOT_VALID,
};
use crate::StoreError;

const COLUMNS: &str = "id, workspace_id, email_lower, role, created_by, token_hash, expires_at, \
                       accepted_at, accepted_member_id, revoked_at, created_at";

fn row_to_invite(row: &sqlx::sqlite::SqliteRow) -> Result<WorkspaceInvite, StoreError> {
    let role: String = row.get("role");
    Ok(WorkspaceInvite {
        id: WorkspaceInviteId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        email: row.get("email_lower"),
        role: WorkspaceInviteRole::parse(&role)
            .ok_or_else(|| StoreError::InvalidInput(format!("unknown invite role {role:?}")))?,
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        token_hash: row.get("token_hash"),
        expires_at: row.get::<DateTime<Utc>, _>("expires_at"),
        accepted_at: row.get::<Option<DateTime<Utc>>, _>("accepted_at"),
        accepted_member_id: row
            .get::<Option<Uuid>, _>("accepted_member_id")
            .map(MemberId),
        revoked_at: row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

/// Mint an invite and its audit row in one transaction (D-A). The inviter
/// must be a live member of the invite's workspace.
pub async fn create_audited(
    pool: &SqlitePool,
    new: NewWorkspaceInvite,
    audit: crate::AuditFor<WorkspaceInvite>,
) -> Result<WorkspaceInvite, StoreError> {
    // One instant for the validation and for `created_at`, so the table's
    // `expires_at > created_at` CHECK agrees with the validation.
    let now = Utc::now();
    let email = validate_new(&new, now)?;
    // The write lock first: this transaction reads the inviter and then
    // writes. A deferred one that read before another connection committed
    // cannot take the write lock afterwards, and SQLite answers `database is
    // locked` at once instead of waiting out busy_timeout.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let inviter_live: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM maidan_members
                        WHERE id = ?1 AND workspace_id = ?2 AND tombstoned_at IS NULL)",
    )
    .bind(new.created_by.0)
    .bind(new.workspace_id.0)
    .fetch_one(&mut *tx)
    .await?;
    if !inviter_live {
        return Err(StoreError::InvalidInput(
            "the inviter must be a live member of the workspace".into(),
        ));
    }
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_workspace_invites
            (id, workspace_id, email_lower, role, created_by, token_hash, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         RETURNING {COLUMNS}"
    ))
    .bind(WorkspaceInviteId::new().0)
    .bind(new.workspace_id.0)
    .bind(&email)
    .bind(new.role.as_str())
    .bind(new.created_by.0)
    .bind(&new.token_hash)
    .bind(new.expires_at)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let invite = row_to_invite(&row)?;
    super::audit::append_scoped(&mut tx, invite.workspace_id, audit(&invite)).await?;
    tx.commit().await?;
    Ok(invite)
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<WorkspaceInvite>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM maidan_workspace_invites
         WHERE workspace_id = ?1 ORDER BY created_at DESC, id DESC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_invite).collect()
}

pub async fn get(pool: &SqlitePool, id: WorkspaceInviteId) -> Result<WorkspaceInvite, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM maidan_workspace_invites WHERE id = ?1"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_invite(&row)
}

/// Only a live invite resolves: unknown, used, revoked and expired hashes
/// all return `NotFound`, so the caller cannot tell them apart.
pub async fn find_live(
    pool: &SqlitePool,
    token_hash: &str,
    now: DateTime<Utc>,
) -> Result<WorkspaceInvite, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM maidan_workspace_invites
         WHERE token_hash = ?1 AND accepted_at IS NULL AND revoked_at IS NULL
           AND julianday(expires_at) > julianday(?2)"
    ))
    .bind(token_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_invite(&row)
}

/// Revoke an unused invite in its workspace, with its audit row. Returns
/// whether this call changed state; nothing is recorded when it did not.
pub async fn revoke_audited(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    id: WorkspaceInviteId,
    audit: maidan_types::NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let revoked = sqlx::query(
        "UPDATE maidan_workspace_invites SET revoked_at = ?3
         WHERE id = ?1 AND workspace_id = ?2 AND revoked_at IS NULL AND accepted_at IS NULL",
    )
    .bind(id.0)
    .bind(workspace_id.0)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if revoked {
        super::audit::append_scoped(&mut tx, workspace_id, audit).await?;
    }
    tx.commit().await?;
    Ok(revoked)
}

/// The role an accepted invite gave this member, if one did.
pub async fn member_role(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Option<WorkspaceInviteRole>, StoreError> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM maidan_workspace_invites WHERE accepted_member_id = ?1",
    )
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(role.as_deref().and_then(WorkspaceInviteRole::parse))
}

/// Accept an invite: consume it, create the human member, link the identity
/// and open the session, all in one commit with the audit rows. Every gate
/// is checked again here under the write lock, so two sign-ins racing one
/// invite cannot both land.
pub async fn accept(
    pool: &SqlitePool,
    accept: AcceptWorkspaceInvite,
    invite_audit: crate::AuditFor<WorkspaceInviteAcceptance>,
    session_audit: crate::AuditFor<MaidanSession>,
) -> Result<WorkspaceInviteAcceptance, StoreError> {
    let email_lower = accept.email.trim().to_lowercase();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // Read after the write lock: an invite that expired while this waited
    // must not be consumed.
    let now = Utc::now();

    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM maidan_oidc_identities
                        WHERE workspace_id = ?1 AND issuer = ?2 AND subject = ?3)",
    )
    .bind(accept.workspace_id.0)
    .bind(&accept.issuer)
    .bind(&accept.subject)
    .fetch_one(&mut *tx)
    .await?;
    if already {
        return Err(StoreError::Conflict(INVITE_ALREADY_MEMBER.into()));
    }

    let mut handle = None;
    for n in 1..=HANDLE_ATTEMPTS {
        let candidate = nth_handle(&accept.handle, n);
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM maidan_members WHERE workspace_id = ?1 AND handle = ?2)",
        )
        .bind(accept.workspace_id.0)
        .bind(&candidate)
        .fetch_one(&mut *tx)
        .await?;
        if !taken {
            handle = Some(candidate);
            break;
        }
    }
    let handle = handle.ok_or_else(|| {
        StoreError::Conflict("no free handle for the invited member; ask for another invite".into())
    })?;

    let member = super::members::create_on(
        &mut tx,
        NewMember {
            workspace_id: accept.workspace_id,
            handle,
            display_name: None,
            kind: MemberKind::Human,
        },
    )
    .await?;

    // The consuming update is the gate: it matches only a live invite in
    // this workspace bound to this email, and only once.
    let row = sqlx::query(&format!(
        "UPDATE maidan_workspace_invites
         SET accepted_at = ?1, accepted_member_id = ?2
         WHERE id = ?3 AND workspace_id = ?4 AND email_lower = ?5
           AND accepted_at IS NULL AND revoked_at IS NULL
           AND julianday(expires_at) > julianday(?1)
         RETURNING {COLUMNS}"
    ))
    .bind(now)
    .bind(member.id.0)
    .bind(accept.invite_id.0)
    .bind(accept.workspace_id.0)
    .bind(&email_lower)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| StoreError::Conflict(INVITE_NOT_VALID.into()))?;
    let invite = row_to_invite(&row)?;

    super::events::append_in_tx(
        &mut tx,
        &Event::MemberJoined {
            occurred_at: now,
            workspace_id: accept.workspace_id,
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
    .bind(accept.workspace_id.0)
    .bind(&accept.issuer)
    .bind(&accept.subject)
    .bind(member.id.0)
    .bind(&accept.email)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let identity = super::oidc::row_to_identity(&identity_row)?;

    let session = super::sessions::create_on(
        &mut tx,
        NewMaidanSession {
            workspace_id: accept.workspace_id,
            member_id: member.id,
            api_token_id: None,
            oidc_identity_id: Some(identity.id),
            expires_at: accept.session_expires_at,
        },
    )
    .await?;

    let outcome = WorkspaceInviteAcceptance {
        invite,
        member,
        identity,
        session,
    };
    super::audit::append_scoped(&mut tx, accept.workspace_id, invite_audit(&outcome)).await?;
    super::audit::append_scoped(
        &mut tx,
        accept.workspace_id,
        session_audit(&outcome.session),
    )
    .await?;
    tx.commit().await?;
    Ok(outcome)
}
