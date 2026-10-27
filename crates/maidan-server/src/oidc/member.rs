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
    link_email_issuer: Option<&str>,
) -> Result<(MemberId, LoginMember), ApiError> {
    if let Ok(identity) = store.get_oidc_identity(workspace_id, issuer, subject).await {
        // The identity row outlives a removed member, so the member is checked
        // again on every sign-in: a tombstoned or missing member is refused,
        // never resolved and never replaced (#1626).
        return match store.get_member(identity.member_id).await {
            Ok(member) if member.workspace_id == workspace_id && member.tombstoned_at.is_none() => {
                Ok((member.id, LoginMember::Existing))
            }
            Ok(_) | Err(maidan_store::StoreError::NotFound) => {
                Err(ApiError::Forbidden(NOT_PROVISIONED.into()))
            }
            Err(err) => Err(err.into()),
        };
    }

    // Linking by email trusts the issuer that asserted the address, so it is
    // scoped to the one issuer named for it. An email verified by any other
    // issuer falls through to provisioning, or is refused (#1625).
    if link_email_issuer == Some(issuer)
        && email_verified
        && let Some(email) = email
        && let Ok(member) = store.get_member_by_handle(workspace_id, email).await
    {
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

    if !auto_provision {
        return Err(ApiError::Forbidden(NOT_PROVISIONED.into()));
    }

    // Auto-provision only ever creates a member. It never attaches the login
    // to a member that already holds the derived handle: the email's local
    // part proves nothing about who owns that handle, so `admin@attacker.com`
    // must not become `admin`. Linking an existing member is the job of
    // `MAIDAN_OIDC_LINK_EMAIL`, which needs the whole verified address from
    // the configured issuer.
    let base = handle_from_claims(subject, email);
    let mut created = None;
    for handle in [base.clone(), disambiguated_handle(&base, subject)] {
        match store.get_member_by_handle(workspace_id, &handle).await {
            Ok(_) => continue,
            Err(maidan_store::StoreError::NotFound) => {}
            Err(err) => return Err(err.into()),
        }
        // A login racing this one can take the handle between the lookup and
        // the insert; that is a taken handle, so move on to the next one.
        match store
            .create_member(NewMember {
                workspace_id,
                handle,
                display_name: email.map(str::to_string),
                kind: MemberKind::Human,
            })
            .await
        {
            Ok(member) => {
                created = Some(member);
                break;
            }
            Err(maidan_store::StoreError::Conflict(_)) => continue,
            Err(err) => return Err(err.into()),
        }
    }
    let Some(member) = created else {
        return Err(ApiError::Forbidden(NOT_PROVISIONED.into()));
    };
    let how = LoginMember::Provisioned;

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

/// The handle a provisioned member takes when the one derived from the email
/// is already held: the base, a dash, and the first twelve handle-safe
/// characters of the subject.
fn disambiguated_handle(base: &str, subject: &str) -> String {
    let suffix: String = subject
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(12)
        .collect();
    format!("{base}-{suffix}")
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
