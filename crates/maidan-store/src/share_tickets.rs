use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use maidan_types::{NewShareTicket, SHARE_TICKET_MAX_ARTIFACTS, SHARE_TICKET_MAX_TTL_SECS};

use crate::StoreError;

pub(crate) fn validate_new(
    new: &NewShareTicket,
    now: DateTime<Utc>,
) -> Result<Vec<String>, StoreError> {
    if new.expires_at <= now || new.expires_at > now + Duration::seconds(SHARE_TICKET_MAX_TTL_SECS)
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
