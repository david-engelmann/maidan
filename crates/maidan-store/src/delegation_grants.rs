use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use maidan_types::{NewApiToken, NewDelegationGrant, DELEGATED_TOKEN_MAX_TTL_SECS};

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

pub(crate) fn validate_exchange(
    new: &NewApiToken,
    grant_capabilities: &[String],
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, StoreError> {
    let expires_at = new
        .expires_at
        .ok_or_else(|| StoreError::InvalidInput("delegated tokens require an expiry".into()))?;
    if expires_at <= now
        || expires_at > now + chrono::Duration::seconds(DELEGATED_TOKEN_MAX_TTL_SECS)
    {
        return Err(StoreError::InvalidInput(
            "delegated token expiry must be in the future and no more than one hour away".into(),
        ));
    }
    if new.capabilities.is_empty() {
        return Err(StoreError::InvalidInput(
            "delegated token must include at least one capability".into(),
        ));
    }
    if let Some(capability) = new
        .capabilities
        .iter()
        .find(|capability| !grant_capabilities.contains(capability))
    {
        return Err(StoreError::InvalidInput(format!(
            "capability {capability} exceeds delegation grant"
        )));
    }
    Ok(expires_at)
}
