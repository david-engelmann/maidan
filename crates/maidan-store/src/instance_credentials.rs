//! Which workspaces a GitHub, Slack or Linear credential serves.
//!
//! A workspace is served on a surface when it stores its own connection for
//! it (a reserved secret, [`ConnectionSurface::secret_name`]), or when the
//! operator opened it, in which case the server's own credential serves it
//! if the server holds one. A workspace opened by sign-up or by the demo's
//! sandbox is never served by the server's credential: its admin is whoever
//! arrived, the destination is theirs to choose, and the operator's
//! credential would do the reading and the writing there.
//!
//! An egress allowlist entry, a Slack channel link, a GitHub issue link and a
//! watched pull request each aim a credential at a destination the workspace
//! names, so each checks this rule. It lives in the store so that REST, the
//! console, MCP and the background workers cannot differ on it.

use maidan_types::{ConnectionSurface, WorkspaceOrigin};

use crate::error::StoreError;

/// What a workspace that is not served on a surface is told.
pub fn refusal(surface: ConnectionSurface) -> String {
    format!(
        "connect {} for this workspace first (Tools, Integrations)",
        surface.label()
    )
}

/// Whether a workspace of this origin, with or without a connection of its
/// own for the surface, is served on it. Whether the server holds a
/// credential of its own is the server's to know; this is the part the store
/// can decide.
pub fn serves(origin: WorkspaceOrigin, has_own_connection: bool) -> bool {
    has_own_connection || origin == WorkspaceOrigin::Operator
}

/// Refuse a workspace that is not served on `surface`.
pub fn ensure_served(
    origin: WorkspaceOrigin,
    has_own_connection: bool,
    surface: ConnectionSurface,
) -> Result<(), StoreError> {
    if serves(origin, has_own_connection) {
        Ok(())
    } else {
        Err(StoreError::InvalidInput(refusal(surface)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_is_served_by_its_own_connection_or_by_the_operators_opening() {
        for surface in ConnectionSurface::ALL {
            assert!(serves(WorkspaceOrigin::Operator, false));
            assert!(ensure_served(WorkspaceOrigin::Operator, false, surface).is_ok());
            for origin in [
                WorkspaceOrigin::Operator,
                WorkspaceOrigin::Signup,
                WorkspaceOrigin::Open,
            ] {
                assert!(serves(origin, true));
            }
            for origin in [WorkspaceOrigin::Signup, WorkspaceOrigin::Open] {
                assert!(!serves(origin, false));
                match ensure_served(origin, false, surface) {
                    Err(StoreError::InvalidInput(message)) => {
                        assert_eq!(message, refusal(surface));
                    }
                    other => panic!("{origin:?} was not refused: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn the_refusal_names_the_surface() {
        assert_eq!(
            refusal(ConnectionSurface::Github),
            "connect GitHub for this workspace first (Tools, Integrations)"
        );
    }
}
