//! The anonymous MCP reader, for a dev instance a client such as ChatGPT
//! developer mode or a claude.ai connector with "No sign-in" can install
//! against before Maidan has an OAuth server.
//!
//! `MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE` names one workspace, which must be
//! called `synthetic-…` so a real team's workspace is never the one exposed. A
//! `POST` to an MCP endpoint with no credential then reads that workspace as
//! its `anonymous-reader` member, with `workspace:read` and only the read-only
//! tools. Every other request still needs a credential. The server refuses to
//! boot with the flag under `MAIDAN_ENV=production` or beside `AUTH_DISABLED`,
//! which is not this mode and would make it meaningless.

use maidan_auth::AuthContext;
use maidan_store::{Store, StoreError};
use maidan_types::{MemberKind, NewMember, WorkspaceId};

use crate::config::ConfigError;

pub const ENV: &str = "MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE";

/// The prefix a workspace's name must carry to be read without a credential.
pub const SYNTHETIC_PREFIX: &str = "synthetic-";

/// The member an anonymous caller reads as.
pub const MEMBER_HANDLE: &str = "anonymous-reader";

/// Whether the flag may be set at all. Pure over its inputs so it is tested
/// without touching the process environment.
pub(crate) fn validate(production: bool, auth_disabled: bool) -> Result<(), ConfigError> {
    if production {
        return Err(ConfigError::Invalid(
            ENV,
            "cannot be set when MAIDAN_ENV=production".into(),
        ));
    }
    if auth_disabled {
        return Err(ConfigError::Invalid(
            ENV,
            "cannot be set with AUTH_DISABLED, which already lets every caller do everything"
                .into(),
        ));
    }
    Ok(())
}

/// The anonymous reader the flag asks for, or `None` when it is unset. A
/// malformed id, a missing workspace or one not named `synthetic-…` stops
/// the boot rather than serving something other than what was asked.
pub async fn from_env(
    store: &dyn Store,
    auth_disabled: bool,
) -> Result<Option<AuthContext>, ConfigError> {
    let Ok(raw) = std::env::var(ENV) else {
        return Ok(None);
    };
    validate(crate::config::is_production(), auth_disabled)?;
    let id = raw
        .trim()
        .parse::<uuid::Uuid>()
        .map(WorkspaceId)
        .map_err(|err| ConfigError::Invalid(ENV, format!("not a workspace id: {err}")))?;
    reader_for(store, id).await.map(Some)
}

pub async fn reader_for(
    store: &dyn Store,
    workspace_id: WorkspaceId,
) -> Result<AuthContext, ConfigError> {
    let workspace = store
        .get_workspace(workspace_id)
        .await
        .map_err(|err| ConfigError::Invalid(ENV, format!("workspace {workspace_id}: {err}")))?;
    if !workspace.name.starts_with(SYNTHETIC_PREFIX) {
        return Err(ConfigError::Invalid(
            ENV,
            format!(
                "workspace `{}` is not named {SYNTHETIC_PREFIX}…, and only a synthetic \
                 workspace is read without a credential",
                workspace.name
            ),
        ));
    }
    let member = match store
        .get_member_by_handle(workspace_id, MEMBER_HANDLE)
        .await
    {
        Ok(member) => member,
        Err(StoreError::NotFound) => store
            .create_member(NewMember {
                workspace_id,
                handle: MEMBER_HANDLE.into(),
                display_name: Some("Anonymous reader (dev)".into()),
                kind: MemberKind::Agent,
            })
            .await
            .map_err(|err| ConfigError::Invalid(ENV, format!("creating {MEMBER_HANDLE}: {err}")))?,
        Err(err) => return Err(ConfigError::Invalid(ENV, err.to_string())),
    };
    Ok(AuthContext::anonymous_reader(member.id, workspace_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_is_refused_in_production_and_beside_auth_disabled() {
        assert!(validate(true, false).is_err());
        assert!(validate(false, true).is_err());
        assert!(validate(false, false).is_ok());
    }
}
