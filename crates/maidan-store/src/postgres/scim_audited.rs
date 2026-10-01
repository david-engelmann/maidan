//! D-A for SCIM provisioning. Creating a user, renaming it, changing whether
//! it is active, and deleting it each commit in one transaction with their records — the
//! member and its SCIM link together, and a deprovision with the revocation of
//! every live token the member holds. A deprovision that cannot finish fails,
//! so the identity provider retries it, rather than reporting success with
//! tokens still live.

use chrono::Utc;
use maidan_types::{AuditScope, Member, MemberId, NewAuditEvent, NewMember, ScimUser, WorkspaceId};
use sqlx::PgPool;
use uuid::Uuid;

use super::{audit, members, scim_users};
use crate::{error::StoreError, AuditFor};

pub async fn provision(
    pool: &PgPool,
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
    pool: &PgPool,
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
        revoke_member_tokens(&mut tx, workspace_id, member_id).await?;
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    event: NewAuditEvent,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    revoke_member_tokens(&mut tx, workspace_id, member_id).await?;
    let deleted = scim_users::delete_on(&mut tx, member_id).await?;
    if deleted {
        audit::append_counted(&mut tx, event).await?;
    }
    tx.commit().await?;
    Ok(deleted)
}

/// Revoke every live token the member holds, one `token.revoke` row each.
async fn revoke_member_tokens(
    conn: &mut sqlx::PgConnection,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    let revoked: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE maidan_api_tokens SET revoked_at = $1
         WHERE workspace_id = $2 AND member_id = $3 AND revoked_at IS NULL
         RETURNING id",
    )
    .bind(Utc::now())
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
    Ok(())
}
