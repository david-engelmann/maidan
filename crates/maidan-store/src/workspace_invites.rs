//! What both backends check before a workspace invite is written or accepted.

use chrono::{DateTime, Duration, Utc};
use maidan_types::{NewWorkspaceInvite, WORKSPACE_INVITE_MAX_TTL_SECS};

use crate::StoreError;

/// The accepting transaction found the identity already has a member in the
/// invite's workspace. The handler answers it with its own words.
pub const INVITE_ALREADY_MEMBER: &str =
    "you already belong to this workspace; sign in instead of using the invite";
/// The invite is gone, used, revoked, expired, or bound to another email or
/// workspace. One message for all of them, so the refusal discloses nothing.
pub const INVITE_NOT_VALID: &str = "the invite is no longer valid";

/// SQLite compares timestamps with `julianday()`, which keeps milliseconds;
/// an expiry must be more than this after `now` for the CHECK to hold.
const EXPIRY_RESOLUTION: Duration = Duration::milliseconds(1);

/// How many numbered handles the accepting transaction tries before it gives
/// up, when the one from the claims is taken.
pub(crate) const HANDLE_ATTEMPTS: u32 = 50;

/// The lowercased email, after the invite's shape is checked.
pub(crate) fn validate_new(
    new: &NewWorkspaceInvite,
    now: DateTime<Utc>,
) -> Result<String, StoreError> {
    if new.expires_at <= now + EXPIRY_RESOLUTION
        || new.expires_at > now + Duration::seconds(WORKSPACE_INVITE_MAX_TTL_SECS)
    {
        return Err(StoreError::InvalidInput(
            "invite expiry must be in the future and no more than 30 days away".into(),
        ));
    }
    if !is_hex_sha256(&new.token_hash) {
        return Err(StoreError::InvalidInput(
            "invite token_hash must be 64 lowercase hex characters".into(),
        ));
    }
    let email = new.email.trim().to_lowercase();
    if !is_plausible_email(&email) {
        return Err(StoreError::InvalidInput(
            "invite email is not an address".into(),
        ));
    }
    Ok(email)
}

fn is_hex_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// One `@` with something on both sides, no empty domain label, no
/// whitespace, at most 320 bytes. A single-label domain passes. The
/// provider's verified email is the real check; this keeps junk out.
pub fn is_plausible_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    email.len() <= 320
        && !local.is_empty()
        && !domain.contains('@')
        && domain.split('.').all(|label| !label.is_empty())
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The `n`th handle to try: the claims' own first, then `handle-2`, `handle-3`.
pub(crate) fn nth_handle(base: &str, n: u32) -> String {
    if n <= 1 {
        base.to_string()
    } else {
        format!("{base}-{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::{MemberId, WorkspaceId, WorkspaceInviteRole};
    use uuid::Uuid;

    fn new(email: &str, expires_in: Duration, hash: &str) -> (NewWorkspaceInvite, DateTime<Utc>) {
        let now = Utc::now();
        (
            NewWorkspaceInvite {
                workspace_id: WorkspaceId(Uuid::nil()),
                email: email.into(),
                role: WorkspaceInviteRole::Member,
                created_by: MemberId(Uuid::nil()),
                token_hash: hash.into(),
                expires_at: now + expires_in,
            },
            now,
        )
    }

    #[test]
    fn an_invite_is_lowercased_and_bounded() {
        let hash = "a".repeat(64);
        let (n, now) = new(" Ann@Example.COM ", Duration::days(7), &hash);
        assert_eq!(
            validate_new(&n, now).ok().as_deref(),
            Some("ann@example.com")
        );
        let (n, now) = new("ann@example.com", Duration::days(31), &hash);
        assert!(validate_new(&n, now).is_err());
        let (n, now) = new("ann@example.com", Duration::zero(), &hash);
        assert!(validate_new(&n, now).is_err());
        let (n, now) = new("not-an-email", Duration::days(1), &hash);
        assert!(validate_new(&n, now).is_err());
        let (n, now) = new("ann@example.com", Duration::days(1), "ABC");
        assert!(validate_new(&n, now).is_err());
    }

    #[test]
    fn an_address_needs_every_domain_label_but_not_a_dot() {
        assert!(is_plausible_email("ann@example.com"));
        assert!(is_plausible_email("ann@localhost"));
        for junk in [
            "a@.",
            "a@",
            "a@example.",
            "a@.example.com",
            "a@example..com",
            "@example.com",
        ] {
            assert!(!is_plausible_email(junk), "{junk} passed");
        }
    }

    #[test]
    fn handles_are_numbered_after_the_first() {
        assert_eq!(nth_handle("ann", 1), "ann");
        assert_eq!(nth_handle("ann", 3), "ann-3");
    }
}
