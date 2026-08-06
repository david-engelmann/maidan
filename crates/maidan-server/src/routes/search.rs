//! Search handler: lexical and semantic message search.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use maidan_auth::{capability::SEARCH_QUERY, AuthContext};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn search_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<maidan_search::SearchHit>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SEARCH_QUERY)?;
    ensure_workspace(&auth, workspace_id)?;
    let filters = maidan_search::SearchFilters {
        author_id: q.author.map(MemberId),
        channel_id: q.channel.map(ChannelId),
        author_kind: q.kind,
    };
    let mut hits = match q.mode {
        SearchMode::Lexical => {
            state
                .search
                .search_messages(workspace_id, &q.q, q.limit, &filters)
                .await?
        }
        SearchMode::Semantic => {
            let embedding = state
                .embedding_provider
                .embed(&q.q)
                .map_err(|e| ApiError::Internal(format!("embedding generation failed: {e}")))?;
            let model = q
                .embedding_model
                .as_deref()
                .unwrap_or_else(|| state.embedding_provider.model_name());
            state
                .search
                .semantic_search(workspace_id, &embedding, q.limit, &filters, model)
                .await?
        }
        SearchMode::Hybrid => {
            let embedding = state
                .embedding_provider
                .embed(&q.q)
                .map_err(|e| ApiError::Internal(format!("embedding generation failed: {e}")))?;
            let model = q
                .embedding_model
                .as_deref()
                .unwrap_or_else(|| state.embedding_provider.model_name());
            let weight = q
                .hybrid_weight
                .unwrap_or(maidan_search::DEFAULT_HYBRID_WEIGHT);
            state
                .search
                .hybrid_search(
                    workspace_id,
                    &q.q,
                    &embedding,
                    q.limit,
                    &filters,
                    model,
                    weight,
                )
                .await?
        }
    };
    // Drop hits in private channels the caller can't access (Cluster 160).
    // Cache the per-channel decision so a result page hits each channel once.
    if !auth.bypass {
        let mut decision: std::collections::HashMap<ChannelId, bool> =
            std::collections::HashMap::new();
        let mut allowed = Vec::with_capacity(hits.len());
        for hit in hits {
            let ok = match decision.get(&hit.channel_id) {
                Some(v) => *v,
                None => {
                    let v = maidan_auth::can_access_channel(
                        state.store.as_ref(),
                        &auth,
                        hit.channel_id,
                    )
                    .await?;
                    decision.insert(hit.channel_id, v);
                    v
                }
            };
            if ok {
                allowed.push(hit);
            }
        }
        hits = allowed;
    }
    let hits = if q.snippet_only {
        hits.into_iter()
            .map(maidan_search::SearchHit::into_snippet_only)
            .collect()
    } else {
        hits
    };
    Ok(Json(hits))
}
