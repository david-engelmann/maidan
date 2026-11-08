//! Releasing every claim a member holds in its workspace at once, for a
//! claimant that restarted: the MCP tool `release_own_claims` and its REST
//! twin `POST /workspaces/{id}/claims/release-own` share the note rule and the
//! answer here, so the two surfaces cannot drift.

use serde::{Deserialize, Serialize};

use crate::ids::ThreadId;

/// The note each release carries when the caller gives none.
pub const RESTART_RELEASE_NOTE: &str = "the claimant restarted";

/// The longest release note, as an approval note: it lands on every released
/// thread's assignment event.
pub const MAX_RELEASE_NOTE_CHARS: usize = 2000;

/// The note to record: the caller's, or [`RESTART_RELEASE_NOTE`] when it gave
/// none or only whitespace. `Err` names the limit when the note is too long.
pub fn release_note(note: Option<String>) -> Result<String, String> {
    match note {
        Some(note) if note.chars().count() > MAX_RELEASE_NOTE_CHARS => {
            Err(format!("note: at most {MAX_RELEASE_NOTE_CHARS} characters"))
        }
        Some(note) if !note.trim().is_empty() => Ok(note),
        _ => Ok(RESTART_RELEASE_NOTE.to_string()),
    }
}

/// The claims a release of the caller's own claims gave back to the queue, in
/// thread id order; empty when it held none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReleasedClaims {
    pub released: Vec<ThreadId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_blank_note_reads_as_a_restart() {
        assert_eq!(release_note(None).as_deref(), Ok(RESTART_RELEASE_NOTE));
        assert_eq!(
            release_note(Some("  \n".into())).as_deref(),
            Ok(RESTART_RELEASE_NOTE)
        );
        assert_eq!(
            release_note(Some("redeployed".into())).as_deref(),
            Ok("redeployed")
        );
    }

    #[test]
    fn a_note_longer_than_the_limit_is_refused() {
        let at_limit = "a".repeat(MAX_RELEASE_NOTE_CHARS);
        assert_eq!(
            release_note(Some(at_limit.clone())).as_deref(),
            Ok(at_limit.as_str())
        );
        assert!(release_note(Some("a".repeat(MAX_RELEASE_NOTE_CHARS + 1))).is_err());
    }
}
