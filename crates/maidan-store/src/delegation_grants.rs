use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use maidan_types::NewDelegationGrant;

use crate::StoreError;

pub(crate) fn validate_new(
    new: &NewDelegationGrant,
    now: DateTime<Utc>,
) -> Result<(Vec<String>, String), StoreError> {
    if new.subject_id == new.delegate_id {
        return Err(StoreError::InvalidInput(
            "delegation subject and delegate must differ".into(),
        ));
    }
    if new.expires_at <= now {
        return Err(StoreError::InvalidInput(
            "delegation grant expiry must be in the future".into(),
        ));
    }
    let purpose = new.purpose.trim();
    if purpose.is_empty() || purpose.len() > 1000 {
        return Err(StoreError::InvalidInput(
            "delegation purpose must contain 1 to 1000 bytes after trimming".into(),
        ));
    }
    let mut capabilities = BTreeSet::new();
    for capability in &new.capabilities {
        let capability = capability.trim();
        if capability.is_empty() || capability.len() > 255 {
            return Err(StoreError::InvalidInput(
                "delegated capabilities must contain 1 to 255 bytes after trimming".into(),
            ));
        }
        capabilities.insert(capability.to_owned());
    }
    if capabilities.is_empty() {
        return Err(StoreError::InvalidInput(
            "delegation grant must include at least one capability".into(),
        ));
    }
    Ok((capabilities.into_iter().collect(), purpose.to_owned()))
}
