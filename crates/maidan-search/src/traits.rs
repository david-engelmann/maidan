use async_trait::async_trait;
use maidan_types::WorkspaceId;

use crate::error::SearchError;
use crate::hit::SearchHit;

/// Backend-agnostic search interface.
///
/// Both lexical and semantic surfaces live on the same trait; backends
/// that do not support a method return [`SearchError::Unsupported`].
#[async_trait]
pub trait Search: Send + Sync {
    /// Lexical search over message bodies within a workspace.
    ///
    /// Tombstoned messages are excluded. Hits are returned ordered by
    /// rank descending; ties broken by `posted_at DESC`. `limit` caps
    /// the result set.
    async fn search_messages(
        &self,
        workspace_id: WorkspaceId,
        query: &str,
        limit: i64,
    ) -> Result<Vec<SearchHit>, SearchError>;
}
