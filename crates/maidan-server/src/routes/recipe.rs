//! Recipe management (Cluster 370.3, Wave 2 #18): create / list / get / delete a
//! recipe blueprint, and **instantiate** one into a parent thread + its DAG
//! children. A recipe spawns threads into its target channel, so the write
//! surfaces are gated on `workspace:write` + access to that channel.

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

use super::{cap, ensure_workspace, publish_stored, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn create_recipe(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateRecipe>,
) -> ApiResult<(StatusCode, Json<Recipe>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;

    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".into()));
    }
    body.spec.validate().map_err(ApiError::BadRequest)?;

    let channel_id = ChannelId(body.channel_id);
    let ctx = maidan_router::resolve_channel_context(state.store.as_ref(), channel_id).await?;
    if ctx.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "channel is not in this workspace".into(),
        ));
    }
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, channel_id).await?;

    let recipe = state
        .store
        .create_recipe(NewRecipe {
            workspace_id,
            channel_id,
            name: body.name.trim().to_string(),
            spec: body.spec,
            created_by: auth.member_id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(recipe)))
}

pub async fn list_recipes(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Recipe>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_recipes(workspace_id).await?))
}

/// Resolve a recipe under `:wid` and authorize the caller: workspace membership
/// (+ channel access for the write paths, checked by the caller).
async fn resolve_recipe(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    recipe_id: RecipeId,
) -> ApiResult<Recipe> {
    ensure_workspace(auth, workspace_id)?;
    let recipe = state.store.get_recipe(recipe_id).await?;
    if recipe.workspace_id != workspace_id {
        return Err(ApiError::NotFound);
    }
    Ok(recipe)
}

pub async fn get_recipe(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, recipe_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<Recipe>> {
    cap(&auth, WORKSPACE_READ)?;
    let recipe = resolve_recipe(
        &state,
        &auth,
        WorkspaceId(workspace_id),
        RecipeId(recipe_id),
    )
    .await?;
    Ok(Json(recipe))
}

pub async fn delete_recipe(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, recipe_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_WRITE)?;
    let recipe_id = RecipeId(recipe_id);
    let recipe = resolve_recipe(&state, &auth, WorkspaceId(workspace_id), recipe_id).await?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, recipe.channel_id).await?;
    if state.store.delete_recipe(recipe_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn instantiate_recipe(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, recipe_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<InstantiateRecipe>,
) -> ApiResult<(StatusCode, Json<RecipeRun>)> {
    cap(&auth, WORKSPACE_WRITE)?;
    let recipe_id = RecipeId(recipe_id);
    let recipe = resolve_recipe(&state, &auth, WorkspaceId(workspace_id), recipe_id).await?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, recipe.channel_id).await?;

    let (run, events) = state
        .store
        .instantiate_recipe(recipe_id, body.params, auth.member_id)
        .await?;
    for stored in events {
        publish_stored(&state, stored).await;
    }
    Ok((StatusCode::CREATED, Json(run)))
}
