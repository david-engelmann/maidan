//! Recipe blueprint CRUD (Cluster 370, Wave 2 #18): the `maidan_recipes` table.
//! The `spec` JSONB column holds the serialized [`RecipeSpec`]; instantiation
//! (Cluster 370.2) lives in `recipe_runs`, not here. See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, NewRecipe, Recipe, RecipeId, RecipeSpec, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "id, workspace_id, channel_id, name, spec, created_by, created_at, updated_at";

fn row_to_recipe(row: &sqlx::postgres::PgRow) -> Result<Recipe, StoreError> {
    let spec: serde_json::Value = row.get("spec");
    Ok(Recipe {
        id: RecipeId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        name: row.get("name"),
        spec: serde_json::from_value::<RecipeSpec>(spec)?,
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}

pub async fn create(pool: &PgPool, new: NewRecipe) -> Result<Recipe, StoreError> {
    let spec = serde_json::to_value(&new.spec)?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_recipes (id, workspace_id, channel_id, name, spec, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
         RETURNING {COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(&new.name)
    .bind(spec)
    .bind(new.created_by.0)
    .fetch_one(pool)
    .await?;
    row_to_recipe(&row)
}

pub async fn get(pool: &PgPool, id: RecipeId) -> Result<Recipe, StoreError> {
    let row = sqlx::query(&format!("SELECT {COLS} FROM maidan_recipes WHERE id = $1"))
        .bind(id.0)
        .fetch_optional(pool)
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_recipe(&row)
}

pub async fn list(pool: &PgPool, workspace_id: WorkspaceId) -> Result<Vec<Recipe>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_recipes WHERE workspace_id = $1 ORDER BY created_at DESC, id"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_recipe).collect()
}

pub async fn delete(pool: &PgPool, id: RecipeId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_recipes WHERE id = $1")
        .bind(id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
