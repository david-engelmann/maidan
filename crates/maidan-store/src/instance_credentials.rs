//! The server's own GitHub, Slack and Linear credentials.
//!
//! There is one set for the whole server, held by its operator. An egress
//! allowlist entry, a Slack channel link and a GitHub issue link each aim
//! those credentials at a destination the workspace names, and a watched pull
//! request is read with them. A workspace the operator opened may do that.
//! One opened by sign-up or by the demo's sandbox may not: its admin is
//! whoever arrived, the destination is theirs to choose, and the operator's
//! credential would do the reading and the writing there.
//!
//! The rule lives in the store so that REST, the console, MCP and the
//! background workers cannot differ on it.

use maidan_types::WorkspaceOrigin;

use crate::error::StoreError;

/// What a workspace the credentials do not serve is told.
pub const REFUSAL: &str = "this server's GitHub, Slack and Linear connections serve only the workspaces its operator opened, and this one was opened by sign-up";

/// Whether a workspace of this origin may aim the server's credentials.
pub fn serves(origin: WorkspaceOrigin) -> bool {
    origin == WorkspaceOrigin::Operator
}

/// Refuse a workspace the credentials do not serve.
pub fn ensure_served(origin: WorkspaceOrigin) -> Result<(), StoreError> {
    if serves(origin) {
        Ok(())
    } else {
        Err(StoreError::InvalidInput(REFUSAL.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_workspace_the_operator_opened_is_served() {
        assert!(serves(WorkspaceOrigin::Operator));
        assert!(ensure_served(WorkspaceOrigin::Operator).is_ok());
        for origin in [WorkspaceOrigin::Signup, WorkspaceOrigin::Open] {
            assert!(!serves(origin));
            match ensure_served(origin) {
                Err(StoreError::InvalidInput(message)) => assert_eq!(message, REFUSAL),
                other => panic!("{origin:?} was not refused: {other:?}"),
            }
        }
    }
}
