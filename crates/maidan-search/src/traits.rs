use async_trait::async_trait;
use maidan_types::{MessageId, WorkspaceId};

use crate::error::SearchError;
use crate::filters::SearchFilters;
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
        filters: &SearchFilters,
    ) -> Result<Vec<SearchHit>, SearchError>;

    /// Store (or replace) the embedding vector for a message under the
    /// given model name. Vector dimension must match the schema
    /// (currently 1024). Implementations that do not support vectors
    /// return [`SearchError::Unsupported`].
    async fn upsert_embedding(
        &self,
        message_id: MessageId,
        model: &str,
        embedding: &[f32],
    ) -> Result<(), SearchError>;

    /// Semantic search by cosine similarity. The query is a caller-
    /// supplied embedding; workspace and optional facets apply. Hits are
    /// returned ordered from most-similar to least-similar; `rank` is
    /// `1.0 - cosine_distance` so higher is more relevant (matching
    /// lexical search semantics).
    async fn semantic_search(
        &self,
        workspace_id: WorkspaceId,
        embedding: &[f32],
        limit: i64,
        filters: &SearchFilters,
        model: &str,
    ) -> Result<Vec<SearchHit>, SearchError>;
}
