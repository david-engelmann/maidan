//! Resolving a [`ThreadLookup`] for a caller, shared by REST and MCP so both
//! answer "not found" for a thread the caller may not read.

use maidan_store::{Store, StoreError};
use maidan_types::{Thread, ThreadLookup, WorkspaceId};

use crate::{AuthContext, AuthError};

/// The thread in `workspace_id` that `lookup` refers to, when the caller may
/// read it. A malformed lookup is `StoreError::InvalidInput`. A thread the
/// caller may not read is `None`, as no thread is, so the answer cannot
/// confirm one exists. The caller has checked `workspace_id` is its own.
pub async fn find_readable_thread(
    store: &dyn Store,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    lookup: &ThreadLookup,
) -> Result<Option<Thread>, AuthError> {
    let reference = lookup.check().map_err(StoreError::InvalidInput)?;
    let Some(thread_id) = store
        .find_thread_by_external_ref(workspace_id, &reference)
        .await?
    else {
        return Ok(None);
    };
    if !crate::can_access_thread(store, auth, thread_id).await? {
        return Ok(None);
    }
    Ok(Some(store.get_thread(thread_id).await?))
}
