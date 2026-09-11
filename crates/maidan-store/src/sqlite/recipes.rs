//! Recipe blueprint CRUD (Cluster 370, SQLite twin of pg 0073). `spec` is a JSON
//! TEXT column holding the serialized [`RecipeSpec`]; timestamps are rfc3339 TEXT.

use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, NewRecipe, Recipe, RecipeId, RecipeSpec, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str = "id, workspace_id, channel_id, name, spec, created_by, created_at, updated_at";

fn row_to_recipe(row: &sqlx::sqlite::SqliteRow) -> Result<Recipe, StoreError> {
    let spec: String = row.get("spec");
    Ok(Recipe {
        id: RecipeId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        name: row.get("name"),
        spec: serde_json::from_str::<RecipeSpec>(&spec)?,
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}

pub async fn create(pool: &SqlitePool, new: NewRecipe) -> Result<Recipe, StoreError> {
    let id = RecipeId::new();
    let now = Utc::now().to_rfc3339();
    let spec = serde_json::to_string(&new.spec)?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_recipes (id, workspace_id, channel_id, name, spec, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING {COLS}"
    ))
    .bind(id.0)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(&new.name)
    .bind(&spec)
    .bind(new.created_by.0)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_recipe(&row)
}

pub async fn get(pool: &SqlitePool, id: RecipeId) -> Result<Recipe, StoreError> {
    let row = sqlx::query(&format!("SELECT {COLS} FROM maidan_recipes WHERE id = ?"))
        .bind(id.0)
        .fetch_optional(pool)
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_recipe(&row)
}

pub async fn list(pool: &SqlitePool, workspace_id: WorkspaceId) -> Result<Vec<Recipe>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_recipes WHERE workspace_id = ? ORDER BY created_at DESC, id"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_recipe).collect()
}

pub async fn delete(pool: &SqlitePool, id: RecipeId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_recipes WHERE id = ?")
        .bind(id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
