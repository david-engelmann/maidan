//! D-A for SCIM provisioning. Creating a user, changing whether it is active,
//! and deleting it each commit in one transaction with their records — the
//! member and its SCIM link together, and a deprovision with the revocation of
//! every live token the member holds. A deprovision that cannot finish fails,
//! so the identity provider retries it, rather than reporting success with
//! tokens still live.

use chrono::Utc;
use maidan_types::{Member, MemberId, NewAuditEvent, NewMember, ScimUser, WorkspaceId};
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

/// `None` when the member has no SCIM link. Deactivating revokes the member's
/// live tokens in the same transaction.
pub async fn set_active(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    external_id: Option<&str>,
    active: bool,
    event: NewAuditEvent,
) -> Result<Option<ScimUser>, StoreError> {
    let mut tx = pool.begin().await?;
    let Some(scim) = scim_users::update_on(&mut tx, member_id, external_id, active).await? else {
        return Ok(None);
    };
    if !active {
        revoke_member_tokens(&mut tx, workspace_id, member_id).await?;
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
