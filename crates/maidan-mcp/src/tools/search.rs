//! Message search (lexical + semantic) tool handler.

use std::sync::Arc;

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
}

fn default_search_limit() -> i64 {
    25
}

pub(super) async fn search_messages(
    search: &Arc<dyn maidan_search::Search>,
    embedding_provider: &Arc<dyn maidan_search::EmbeddingProvider>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SearchMessagesArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    let filters = maidan_search::SearchFilters {
        author_id: a.author_id.map(maidan_types::MemberId),
        channel_id: a.channel_id.map(maidan_types::ChannelId),
        author_kind: a.kind,
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
    };
    Ok(content_json(&hits))
}
