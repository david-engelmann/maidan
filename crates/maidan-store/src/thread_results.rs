//! What both backends share about result writes.

use maidan_types::ClaimLeaseId;

use crate::StoreError;

/// The refusal for a result write whose lease is not the thread's live one.
pub(crate) fn claim_lost(lease_id: ClaimLeaseId) -> StoreError {
    StoreError::ClaimLost(format!(
        "claim lease {} is not this thread's live claim (it lapsed and was taken over, or was released); nothing was stored",
        lease_id.0
    ))
}
