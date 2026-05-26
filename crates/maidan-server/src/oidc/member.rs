use maidan_store::Store;
use maidan_types::{Member, MemberId, MemberKind, NewMember, NewOidcIdentity, WorkspaceId};

use crate::error::ApiError;

pub fn handle_from_claims(subject: &str, email: Option<&str>) -> String {
    if let Some(email) = email {
        let local = email.split('@').next().unwrap_or(email);
        let sanitized: String = local
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        if !sanitized.is_empty() {
            return sanitized;
        }
    }
    let short = subject.chars().take(12).collect::<String>();
    format!("oidc-{short}")
}

pub async fn resolve_member_for_login(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    issuer: &str,
    subject: &str,
    email: Option<&str>,
    email_verified: bool,
    auto_provision: bool,
    link_email: bool,
) -> Result<MemberId, ApiError> {
    if let Ok(identity) = store.get_oidc_identity(workspace_id, issuer, subject).await {
        return Ok(identity.member_id);
    }

    if link_email && email_verified {
        if let Some(email) = email {
            if let Ok(member) = store.get_member_by_handle(workspace_id, email).await {
                store
                    .upsert_oidc_identity(NewOidcIdentity {
                        workspace_id,
                        issuer: issuer.to_string(),
                        subject: subject.to_string(),
                        member_id: member.id,
                        email: Some(email.to_string()),
                    })
                    .await?;
                return Ok(member.id);
            }
        }
    }

    if !auto_provision {
        return Err(ApiError::Forbidden(
            "OIDC user is not provisioned in this workspace".into(),
        ));
    }

    let handle = handle_from_claims(subject, email);
    let member = match store.get_member_by_handle(workspace_id, &handle).await {
        Ok(m) => m,
        Err(maidan_store::StoreError::NotFound) => {
            store
                .create_member(NewMember {
                    workspace_id,
                    handle,
                    display_name: email.map(str::to_string),
                    kind: MemberKind::Human,
                })
                .await?
        }
        Err(err) => return Err(err.into()),
    };

    store
        .upsert_oidc_identity(NewOidcIdentity {
            workspace_id,
            issuer: issuer.to_string(),
            subject: subject.to_string(),
            member_id: member.id,
            email: email.map(str::to_string),
        })
        .await?;

    Ok(member.id)
}

pub async fn touch_identity(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    issuer: &str,
    subject: &str,
    member_id: MemberId,
    email: Option<&str>,
) -> Result<(), ApiError> {
    store
        .upsert_oidc_identity(NewOidcIdentity {
            workspace_id,
            issuer: issuer.to_string(),
            subject: subject.to_string(),
            member_id,
            email: email.map(str::to_string),
        })
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub fn member_kind_is_human(member: &Member) -> bool {
    member.kind == MemberKind::Human
}
