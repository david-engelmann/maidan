//! The anonymous MCP reader, for a dev instance that a client which installs
//! with no sign-in can use before Maidan has an OAuth server.
//!
//! `MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE` names one workspace, which must be
//! called `synthetic-…` so a real team's workspace is never the one exposed. A
//! `POST` to an MCP endpoint with no credential then reads that workspace as no
//! member at all, with `workspace:read` and only the read-only tools, so it sees
//! what is public there and nothing a membership grants. Every other request
//! still needs a credential. The server refuses to boot with the flag under
//! `MAIDAN_ENV=production` or beside `AUTH_DISABLED`, which is not this mode and
//! would make it meaningless.

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{MemberId, WorkspaceId};

use crate::config::ConfigError;

pub const ENV: &str = "MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE";

/// The prefix a workspace's name must carry to be read without a credential.
pub const SYNTHETIC_PREFIX: &str = "synthetic-";

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
    // No member at all: an id that belongs to nobody, fresh each boot, so no
    // channel membership, DM, grant or role can ever reach it. The foreign
    // keys refuse to attach anything to a member that does not exist, and this
    // caller only reads, so nothing is ever written under the id.
    let nobody = MemberId(uuid::Uuid::new_v4());
    Ok(AuthContext::anonymous_reader(nobody, workspace_id))
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
