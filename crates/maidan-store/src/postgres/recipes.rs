//! Recipe blueprint CRUD (Cluster 370, Wave 2 #18): the `maidan_recipes` table.
//! The `spec` JSONB column holds the serialized [`RecipeSpec`]; instantiation
//! (Cluster 370.2) lives in `recipe_runs`, not here. See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, Event, MemberId, NewRecipe, Recipe, RecipeId, RecipeRun, RecipeRunId, RecipeSpec,
    StoredEvent, Thread, ThreadId, WorkspaceId,
};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::{events, threads};
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

const RUN_COLS: &str =
    "id, recipe_id, workspace_id, root_thread_id, params, spec_snapshot, created_by, created_at";

fn row_to_run(row: &sqlx::postgres::PgRow) -> Result<RecipeRun, StoreError> {
    let snapshot: serde_json::Value = row.get("spec_snapshot");
    Ok(RecipeRun {
        id: RecipeRunId(row.get::<Uuid, _>("id")),
        recipe_id: RecipeId(row.get::<Uuid, _>("recipe_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        root_thread_id: ThreadId(row.get::<Uuid, _>("root_thread_id")),
        params: row.get("params"),
        spec_snapshot: serde_json::from_value::<RecipeSpec>(snapshot)?,
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

/// Insert one thread (parent or child) + its `ThreadCreated` event in the tx.
async fn insert_thread(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel_id: ChannelId,
    workspace_id: WorkspaceId,
    parent: Option<ThreadId>,
    title: &str,
) -> Result<(Thread, StoredEvent), StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title)
         VALUES ($1, $2, $3, $4)
         RETURNING id, channel_id, parent_thread_id, title, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id",
    )
    .bind(Uuid::new_v4())
    .bind(channel_id.0)
    .bind(parent.map(|p| p.0))
    .bind(title)
    .fetch_one(&mut **tx)
    .await?;
    let thread = threads::row_to_thread(&row)?;
    let event = Event::ThreadCreated {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread: thread.clone(),
    };
    let stored = events::append_in_tx(tx, &event).await?;
    Ok((thread, stored))
}

pub async fn instantiate(
    pool: &PgPool,
    recipe_id: RecipeId,
    params: serde_json::Value,
    actor: MemberId,
) -> Result<(RecipeRun, Vec<StoredEvent>), StoreError> {
    let recipe = get(pool, recipe_id).await?;
    recipe
        .spec
        .validate_params(&params)
        .map_err(StoreError::InvalidInput)?;

    let mut tx = pool.begin().await?;
    let mut events_out = Vec::new();

    let (parent, parent_ev) = insert_thread(
        &mut tx,
        recipe.channel_id,
        recipe.workspace_id,
        None,
        &recipe.name,
    )
    .await?;
    events_out.push(parent_ev);

    // Create each child; remember key → thread id for the dependency wiring.
    let mut child_ids: HashMap<&str, ThreadId> = HashMap::new();
    for child in &recipe.spec.children {
        let (thread, ev) = insert_thread(
            &mut tx,
            recipe.channel_id,
            recipe.workspace_id,
            Some(parent.id),
            &child.title,
        )
        .await?;
        events_out.push(ev);
        child_ids.insert(child.key.as_str(), thread.id);
        for skill in &child.required_skills {
            sqlx::query(
                "INSERT INTO maidan_thread_required_skills (thread_id, skill)
                 VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(thread.id.0)
            .bind(skill)
            .execute(&mut *tx)
            .await?;
        }
    }

    // Wire the DAG: each child depends on its siblings; the parent depends on
    // every child so it lands last. `spec.validate` (at create) proved the child
    // graph acyclic, and the parent is a fresh node with no dependents.
    for child in &recipe.spec.children {
        let child_id = child_ids[child.key.as_str()];
        for dep in &child.depends_on {
            insert_dep(&mut tx, child_id, child_ids[dep.as_str()]).await?;
        }
        insert_dep(&mut tx, parent.id, child_id).await?;
    }

    let snapshot = serde_json::to_value(&recipe.spec)?;
    let run_row = sqlx::query(&format!(
        "INSERT INTO maidan_recipe_runs (id, recipe_id, workspace_id, root_thread_id, params, spec_snapshot, created_by, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
         RETURNING {RUN_COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(recipe.id.0)
    .bind(recipe.workspace_id.0)
    .bind(parent.id.0)
    .bind(&params)
    .bind(snapshot)
    .bind(actor.0)
    .fetch_one(&mut *tx)
    .await?;
    let run = row_to_run(&run_row)?;

    tx.commit().await?;
    Ok((run, events_out))
}

async fn insert_dep(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    depends_on: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_dependencies (thread_id, depends_on_thread_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(depends_on.0)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn get_run(pool: &PgPool, id: RecipeRunId) -> Result<RecipeRun, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {RUN_COLS} FROM maidan_recipe_runs WHERE id = $1"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_run(&row)
}

pub async fn latest_run(
    pool: &PgPool,
    recipe_id: RecipeId,
) -> Result<Option<RecipeRun>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {RUN_COLS} FROM maidan_recipe_runs WHERE recipe_id = $1 ORDER BY created_at DESC, id LIMIT 1"
    ))
    .bind(recipe_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_run).transpose()
}
