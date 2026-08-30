//! Shared-glossary management (Cluster 322, fidelity arc): set / get / list /
//! delete a workspace's canonical `term -> definition` (+ aliases). Workspace-
//! scoped; the anti-drift pin and the target of 319's `defines` reference
//! relation. `set` upserts (Cluster 321 store). Surfaces the 321 foundation over
//! REST; MCP twins live in `maidan-mcp`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

/// `PUT /workspaces/:wid/glossary/:term` — define (or redefine) a term. Upserts on
/// `(workspace, term)`; `created_by` is the acting member.
pub async fn set_glossary_term(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, term)): Path<(uuid::Uuid, String)>,
    ApiJson(body): ApiJson<SetGlossaryTerm>,
) -> ApiResult<Json<GlossaryTerm>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    if term.trim().is_empty() {
        return Err(ApiError::BadRequest("term must not be empty".into()));
    }
    if body.definition.trim().is_empty() {
        return Err(ApiError::BadRequest("definition must not be empty".into()));
    }
    let saved = state
        .store
        .set_glossary_term(NewGlossaryTerm {
            workspace_id,
            term: term.trim().to_string(),
            definition: body.definition,
            aliases: body.aliases.unwrap_or_default(),
            created_by: auth.member_id,
        })
        .await?;
    Ok(Json(saved))
}

/// `GET /workspaces/:wid/glossary` — all defined terms, ordered by term.
pub async fn list_glossary_terms(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<GlossaryTerm>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    Ok(Json(state.store.list_glossary_terms(workspace_id).await?))
}

/// `GET /workspaces/:wid/glossary/:term` — one term's definition, `404` if undefined.
pub async fn get_glossary_term(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, term)): Path<(uuid::Uuid, String)>,
) -> ApiResult<Json<GlossaryTerm>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    match state.store.get_glossary_term(workspace_id, &term).await? {
        Some(t) => Ok(Json(t)),
        None => Err(ApiError::NotFound),
    }
}

/// `DELETE /workspaces/:wid/glossary/:term` — remove a definition. `204`/`404`.
pub async fn delete_glossary_term(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, term)): Path<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    if state
        .store
        .delete_glossary_term(workspace_id, &term)
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
