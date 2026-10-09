//! D-A for SCIM provisioning. Creating a user, renaming it, changing whether
//! it is active, and deleting it each commit in one transaction with their records — the
//! member and its SCIM link together, and a deprovision with the revocation of
//! every live token the member holds. A deprovision that cannot finish fails,
//! so the identity provider retries it, rather than reporting success with
//! tokens still live.

use chrono::Utc;
use maidan_types::{AuditScope, Member, MemberId, NewAuditEvent, NewMember, ScimUser, WorkspaceId};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{audit, members, scim_users};
use crate::{error::StoreError, AuditFor};

pub async fn provision(
    pool: &SqlitePool,
    new: NewMember,
    external_id: Option<&str>,
    active: bool,
    audit_for: AuditFor<(Member, ScimUser)>,
) -> Result<(Member, ScimUser), StoreError> {
    let mut tx = pool.begin().await?;
    let member = members::create_on(&mut tx, new).await?;
    let scim =
        scim_users::create_on(&mut tx, member.id, member.workspace_id, external_id, active).await?;
    let provisioned = (member, scim);
    audit::append_counted(&mut tx, audit_for(&provisioned)).await?;
    tx.commit().await?;
    Ok(provisioned)
}

/// `None` when the workspace has no such SCIM user. A rename changes the
/// member's handle and nothing else: the id, and everything attributed to it,
/// stay. Deactivating revokes the member's live tokens and releases their
/// claims, charging each claim's worked wall time, in the same transaction.
pub async fn update_user(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    user_name: Option<&str>,
    external_id: Option<&str>,
    active: bool,
    event: NewAuditEvent,
) -> Result<Option<ScimUser>, StoreError> {
    let mut tx = pool.begin().await?;
    let Some(scim) = scim_users::update_on(&mut tx, member_id, external_id, active).await? else {
        return Ok(None);
    };
    if scim.workspace_id != workspace_id {
        // Dropping the transaction undoes the update above.
        return Ok(None);
    }
    if let Some(handle) = user_name {
        members::rename_on(&mut tx, workspace_id, member_id, handle).await?;
    }
    if !active {
        end_member_authority(&mut tx, workspace_id, member_id).await?;
        // Deactivation ends the member's claims. Charge the time each one
        // worked, then release it, in this same transaction.
        super::threads::release_member_claims_in_tx(&mut tx, member_id).await?;
    }
    audit::append_counted(&mut tx, event).await?;
    tx.commit().await?;
    Ok(Some(scim))
}

/// Revoke the member's live tokens and remove its SCIM link. The member row
/// stays, for message authorship. `false` when there was no link.
pub async fn deprovision(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    end_member_authority(&mut tx, workspace_id, member_id).await?;
    let deleted = scim_users::delete_on(&mut tx, member_id).await?;
    if deleted {
        audit::append_counted(&mut tx, event).await?;
    }
    tx.commit().await?;
    Ok(deleted)
}

/// End every authority the member holds in the workspace, each with its own
/// audit row: its live tokens, the delegation grants it holds as delegate, and
/// its browser sessions.
async fn end_member_authority(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    let revoked: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE maidan_api_tokens SET revoked_at = ?
         WHERE workspace_id = ? AND member_id = ? AND revoked_at IS NULL
         RETURNING id",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(&mut *conn)
    .await?;
    for token_id in revoked {
        audit::append_counted(
            &mut *conn,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: None,
                action: "token.revoke".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(token_id),
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "subject_member_id": member_id.0,
                    "reason": "scim_deprovision",
                }),
            },
        )
        .await?;
    }
    // A grant this member holds as delegate lends someone else's authority.
    // Its delegated tokens belong to the grant's subject, so the revoke above
    // misses them; revoking the grant ends them and every token minted from
    // them.
    let grants: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE maidan_delegation_grants SET revoked_at = ?
         WHERE workspace_id = ? AND delegate_id = ? AND revoked_at IS NULL
         RETURNING id, subject_id",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(&mut *conn)
    .await?;
    for (grant_id, subject_id) in grants {
        audit::append_counted(
            &mut *conn,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: None,
                action: "delegation_grant.revoke".into(),
                target_kind: Some("delegation_grant".into()),
                target_id: Some(grant_id),
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "subject_id": subject_id,
                    "delegate_id": member_id.0,
                    "reason": "scim_deprovision",
                }),
            },
        )
        .await?;
    }
    // A browser session the person signed in to has no token to revoke.
    let sessions: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM maidan_sessions WHERE workspace_id = ? AND member_id = ? RETURNING id",
    )
    .bind(workspace_id.0)
    .bind(member_id.0)
    .fetch_all(&mut *conn)
    .await?;
    if !sessions.is_empty() {
        audit::append_counted(
            &mut *conn,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: None,
                action: "session.delete".into(),
                target_kind: Some("member".into()),
                target_id: Some(member_id.0),
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "sessions": sessions.len(),
                    "reason": "scim_deprovision",
                }),
            },
        )
        .await?;
    }
    Ok(())
}
