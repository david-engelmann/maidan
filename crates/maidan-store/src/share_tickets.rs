use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use maidan_types::{NewShareTicket, SHARE_TICKET_MAX_ARTIFACTS, SHARE_TICKET_MAX_TTL_SECS};

use crate::StoreError;

/// SQLite compares timestamps as `julianday()`, which rounds to the
/// millisecond. An expiry must be more than this after `now` for the table's
/// `expires_at > created_at` CHECK to hold on both backends.
const EXPIRY_RESOLUTION: Duration = Duration::milliseconds(1);

pub(crate) fn validate_new(
    new: &NewShareTicket,
    now: DateTime<Utc>,
) -> Result<Vec<String>, StoreError> {
    if new.expires_at <= now + EXPIRY_RESOLUTION
        || new.expires_at > now + Duration::seconds(SHARE_TICKET_MAX_TTL_SECS)
    {
        return Err(StoreError::InvalidInput(
            "share ticket expiry must be in the future and no more than 48 hours away".into(),
        ));
    }
    if new.token_hash.len() != 64
        || !new
            .token_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StoreError::InvalidInput(
            "share ticket token_hash must be 64 lowercase hex characters".into(),
        ));
    }
    if new.artifact_shas.len() > SHARE_TICKET_MAX_ARTIFACTS {
        return Err(StoreError::InvalidInput(format!(
            "a share ticket may contain at most {SHARE_TICKET_MAX_ARTIFACTS} artifacts"
        )));
    }
    let mut artifacts = BTreeSet::new();
    for sha in &new.artifact_shas {
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(StoreError::InvalidInput(
                "share ticket artifact SHA-256 values must be 64 lowercase hex characters".into(),
            ));
        }
        artifacts.insert(sha.clone());
    }
    Ok(artifacts.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use maidan_types::{ChannelId, MemberId, WorkspaceId};
    use uuid::Uuid;

    use super::*;

    fn expiring_at(expires_at: DateTime<Utc>) -> NewShareTicket {
        NewShareTicket {
            workspace_id: WorkspaceId(Uuid::nil()),
            channel_id: ChannelId(Uuid::nil()),
            owner_id: MemberId(Uuid::nil()),
            created_by: MemberId(Uuid::nil()),
            token_hash: "a".repeat(64),
            expires_at,
            artifact_shas: Vec::new(),
        }
    }

    #[test]
    fn an_expiry_is_accepted_only_past_the_database_resolution() {
        // Late in a second, so a sub-second expiry lands in the next one: the
        // case the old whole-second CHECK rejected after validation passed.
        let now =
            Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap() + Duration::milliseconds(999);
        for (after, accepted) in [
            (Duration::milliseconds(-1), false),
            (Duration::zero(), false),
            (Duration::milliseconds(1), false),
            (Duration::microseconds(1_001), true),
            (Duration::milliseconds(2), true),
            (Duration::seconds(SHARE_TICKET_MAX_TTL_SECS), true),
            (
                Duration::seconds(SHARE_TICKET_MAX_TTL_SECS) + Duration::milliseconds(1),
                false,
            ),
        ] {
            let result = validate_new(&expiring_at(now + after), now);
            assert_eq!(result.is_ok(), accepted, "expiry {after} after now");
            if !accepted {
                assert!(matches!(result, Err(StoreError::InvalidInput(_))));
            }
        }
    }
}
