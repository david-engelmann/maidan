//! Thread and workspace context packs. The bytes are built in
//! `maidan_store::context_pack` and laid out in `maidan_types`, so REST and
//! MCP serialize one type.

pub use maidan_store::context_pack::{
    build_channel_boot, build_thread_context as build_thread_context_store,
    build_workspace_context as build_workspace_context_store, ThreadContextLimits,
};
pub use maidan_types::{
    BootPack, ContextDelta, ContextPrefix, ContextTail, MessageEditView, SplitContext,
    ThreadContext, WorkspaceContext,
};

use maidan_store::Store;
use maidan_types::*;

use crate::error::ApiError;

pub async fn build_thread_context(
    store: &dyn Store,
    thread_id: ThreadId,
    limits: ThreadContextLimits,
) -> Result<ThreadContext, ApiError> {
    build_thread_context_store(store, thread_id, limits)
        .await
        .map_err(Into::into)
}

pub async fn build_workspace_context(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_limit: i64,
    thread_cursor: Option<ThreadId>,
    limits: ThreadContextLimits,
) -> Result<WorkspaceContext, ApiError> {
    build_workspace_context_store(store, workspace_id, thread_limit, thread_cursor, limits)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metadata_extractor_collects_sha_fields() {
        let meta = json!({
            "artifact_sha256": "aa".repeat(32),
            "artifacts": [{"sha256": "bb".repeat(32)}]
        });
        let shas = artifact_shas_from_metadata(&meta);
        assert_eq!(shas.len(), 2);
    }
}
