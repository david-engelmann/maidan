//! Message search (lexical + semantic) tool handler.

use std::collections::HashMap;
use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use serde::Deserialize;
use serde_json::Value;

use maidan_types::*;

use super::content_json;
use crate::error::McpError;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchMessagesMode {
    #[default]
    Lexical,
    Semantic,
    Hybrid,
}

#[derive(Deserialize)]
struct SearchMessagesArgs {
    workspace_id: uuid::Uuid,
    query: String,
    #[serde(default)]
    mode: SearchMessagesMode,
    #[serde(default = "default_search_limit")]
    limit: i64,
    author_id: Option<uuid::Uuid>,
    channel_id: Option<uuid::Uuid>,
    kind: Option<maidan_types::MemberKind>,
    embedding_model: Option<String>,
    hybrid_weight: Option<f64>,
    /// Drop full `body` from each hit, keeping only the snippet (Cluster 175,
    /// token round 3) — parity with the REST `snippet_only` param (Cluster 152).
    #[serde(default)]
    snippet_only: bool,
}

fn default_search_limit() -> i64 {
    25
}

pub(super) async fn search_messages(
    search: &Arc<dyn maidan_search::Search>,
    embedding_provider: &Arc<dyn maidan_search::EmbeddingProvider>,
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SearchMessagesArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    // Exclude private channels the caller can't see at the query level (Cluster
    // 200) so they don't crowd out `limit`; the post-filter below is still the
    // authoritative, DM-aware check.
    let deny_channels =
        maidan_auth::private_channel_deny_set(store.as_ref(), auth, workspace_id).await?;
    let filters = maidan_search::SearchFilters {
        author_id: a.author_id.map(maidan_types::MemberId),
        channel_id: a.channel_id.map(maidan_types::ChannelId),
        author_kind: a.kind,
        deny_channels,
    };
    let hits = match a.mode {
        SearchMessagesMode::Lexical => {
            search
                .search_messages(workspace_id, &a.query, a.limit, &filters)
                .await?
        }
        SearchMessagesMode::Semantic => {
            let embedding = embedding_provider
                .embed(&a.query)
                .map_err(|e| McpError::Internal(format!("embedding generation failed: {e}")))?;
            let model = a
                .embedding_model
                .as_deref()
                .unwrap_or_else(|| embedding_provider.model_name());
            search
                .semantic_search(workspace_id, &embedding, a.limit, &filters, model)
                .await?
        }
        SearchMessagesMode::Hybrid => {
            let embedding = embedding_provider
                .embed(&a.query)
                .map_err(|e| McpError::Internal(format!("embedding generation failed: {e}")))?;
            let model = a
                .embedding_model
                .as_deref()
                .unwrap_or_else(|| embedding_provider.model_name());
            let weight = a
                .hybrid_weight
                .unwrap_or(maidan_search::DEFAULT_HYBRID_WEIGHT);
            search
                .hybrid_search(
                    workspace_id,
                    &a.query,
                    &embedding,
                    a.limit,
                    &filters,
                    model,
                    weight,
                )
                .await?
        }
    };
    // Drop hits the caller can't access — thread-keyed + DM-participant-aware
    // (Cluster 180; the earlier channel-keyed filter exempted `__dm__` and
    // leaked DM content). Cache the per-thread decision.
    let hits = if auth.bypass {
        hits
    } else {
        let mut decision: HashMap<ThreadId, bool> = HashMap::new();
        let mut allowed = Vec::with_capacity(hits.len());
        for hit in hits {
            let ok = match decision.get(&hit.thread_id) {
                Some(v) => *v,
                None => {
                    let v =
                        maidan_auth::can_access_thread(store.as_ref(), auth, hit.thread_id).await?;
                    decision.insert(hit.thread_id, v);
                    v
                }
            };
            if ok {
                allowed.push(hit);
            }
        }
        allowed
    };
    let hits = if a.snippet_only {
        hits.into_iter()
            .map(maidan_search::SearchHit::into_snippet_only)
            .collect()
    } else {
        hits
    };
    Ok(content_json(&hits))
}
