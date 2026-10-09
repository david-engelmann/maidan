use maidan_store::Store;
use maidan_types::{MemberId, MemberKind, NewMember, NewOidcIdentity, OidcIdentity, WorkspaceId};

use crate::error::ApiError;

/// The refusal for a sign-in to a workspace the person has no member in. A
/// workspace id that does not exist gets the same words, so the callback tells
/// nobody which ids are real.
pub const NOT_PROVISIONED: &str = "OIDC user is not provisioned in this workspace";

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

/// How a sign-in found its member, recorded on the `session.create` row: a
/// sign-in that provisions a member or links an identity changes more than the
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMember {
    /// The identity was already linked to the member.
    Existing,
    /// The identity was linked to a member that already existed.
    Linked,
    /// The member was created for this identity.
    Provisioned,
}

impl LoginMember {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Existing => "existing",
            Self::Linked => "linked",
            Self::Provisioned => "provisioned",
        }
    }
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
) -> Result<(MemberId, LoginMember), ApiError> {
    if let Ok(identity) = store.get_oidc_identity(workspace_id, issuer, subject).await {
        return Ok((identity.member_id, LoginMember::Existing));
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
                return Ok((member.id, LoginMember::Linked));
            }
        }
    }

    if !auto_provision {
        return Err(ApiError::Forbidden(NOT_PROVISIONED.into()));
    }

    let handle = handle_from_claims(subject, email);
    let (member, how) = match store.get_member_by_handle(workspace_id, &handle).await {
        Ok(m) => (m, LoginMember::Linked),
        Err(maidan_store::StoreError::NotFound) => {
            let created = store
                .create_member(NewMember {
                    workspace_id,
                    handle,
                    display_name: email.map(str::to_string),
                    kind: MemberKind::Human,
                })
                .await?;
            (created, LoginMember::Provisioned)
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

    Ok((member.id, how))
}

pub async fn touch_identity(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    issuer: &str,
    subject: &str,
    member_id: MemberId,
    email: Option<&str>,
) -> Result<OidcIdentity, ApiError> {
    Ok(store
        .upsert_oidc_identity(NewOidcIdentity {
            workspace_id,
            issuer: issuer.to_string(),
            subject: subject.to_string(),
            member_id,
            email: email.map(str::to_string),
        })
        .await?)
}
