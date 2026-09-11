//! Recipe MCP tools (Cluster 370.4, Wave 2 #18): an agent creates + inspects
//! recipe blueprints and **instantiates** one into a parent thread + its DAG
//! children. The REST twin is Cluster 370.3. Writes are `workspace:write` +
//! target-channel access; the list is `workspace:read`, filtered to channels the
//! caller can access.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_router::resolve_channel_context;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct CreateRecipeArgs {
    channel_id: uuid::Uuid,
    name: String,
    spec: RecipeSpec,
}

/// Create a recipe blueprint (Cluster 370.4). Channel access is enforced
/// pre-dispatch (the `channel_id` arg); the recipe is owned by the caller
/// (`created_by = auth.member_id`) and scoped to the caller's workspace.
pub(super) async fn create_recipe(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CreateRecipeArgs = serde_json::from_value(args.clone())?;
    if a.name.trim().is_empty() {
        return Err(McpError::InvalidParams("name must not be empty".into()));
    }
    a.spec.validate().map_err(McpError::InvalidParams)?;
    let channel_id = ChannelId(a.channel_id);
    let ctx = resolve_channel_context(store.as_ref(), channel_id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    if !auth.bypass && ctx.workspace_id != auth.workspace_id {
        return Err(McpError::InvalidParams(
            "channel is not in the caller's workspace".into(),
        ));
    }
    let recipe = store
        .create_recipe(NewRecipe {
            workspace_id: ctx.workspace_id,
            channel_id,
            name: a.name.trim().to_string(),
            spec: a.spec,
            created_by: auth.member_id,
        })
        .await?;
    Ok(content_json(&recipe))
}

/// List the caller's workspace's recipes (Cluster 370.4), filtered to the
/// channels the caller can access — a workspace-scoped aggregate read the
/// pre-dispatch gate can't cover (mirrors `list_task_schedules`).
pub(super) async fn list_recipes(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let recipes = store.list_recipes(auth.workspace_id).await?;
    if auth.bypass {
        return Ok(content_json(&recipes));
    }
    let mut visible = Vec::with_capacity(recipes.len());
    for r in recipes {
        if maidan_auth::can_access_channel(store.as_ref(), auth, r.channel_id).await? {
            visible.push(r);
        }
    }
    Ok(content_json(&visible))
}

#[derive(Deserialize)]
struct InstantiateArgs {
    recipe_id: uuid::Uuid,
    #[serde(default)]
    params: Value,
}

/// Instantiate a recipe into a parent thread + its DAG children (Cluster 370.4)
/// and publish each `ThreadCreated`. The `recipe_id` is not a channel, so the
/// pre-dispatch gate can't cover it — the handler resolves the recipe's channel
/// and enforces access inline (mirrors the REST route).
pub(super) async fn instantiate_recipe(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: InstantiateArgs = serde_json::from_value(args.clone())?;
    let recipe_id = RecipeId(a.recipe_id);
    let recipe = server.store.get_recipe(recipe_id).await?;
    if !auth.bypass && recipe.workspace_id != auth.workspace_id {
        return Err(McpError::NotFound);
    }
    maidan_auth::ensure_channel_access(server.store.as_ref(), auth, recipe.channel_id).await?;

    let (run, events) = server
        .store
        .instantiate_recipe(recipe_id, a.params, auth.member_id)
        .await?;
    for stored in &events {
        server.publish_stored(stored).await;
    }
    Ok(content_json(&run))
}
