use chrono::{DateTime, Utc};
use maidan_types::{GlossaryTerm, MemberId, NewGlossaryTerm, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a workspace's canonical definition of a term (Cluster 321) — see
/// the Postgres twin. `aliases` is stored as a JSON array in a TEXT column, so the
/// bind serializes and `row_to_term` parses (fallible, unlike the JSONB Postgres
/// side). Re-setting the same `(workspace_id, term)` overwrites the
/// definition/aliases and bumps `updated_at`, keeping `created_by`/`created_at`.
pub async fn set(pool: &SqlitePool, new: &NewGlossaryTerm) -> Result<GlossaryTerm, StoreError> {
    let aliases = serde_json::to_string(&new.aliases)?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_glossary_terms
             (id, workspace_id, term, definition, aliases, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, term) DO UPDATE SET
             definition = excluded.definition,
             aliases = excluded.aliases,
             updated_at = excluded.updated_at
         RETURNING id, workspace_id, term, definition, aliases, created_by, created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(new.workspace_id.0)
    .bind(&new.term)
    .bind(&new.definition)
    .bind(&aliases)
    .bind(new.created_by.0)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_term(&row)
}

pub async fn get(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    term: &str,
) -> Result<Option<GlossaryTerm>, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, term, definition, aliases, created_by, created_at, updated_at
         FROM maidan_glossary_terms WHERE workspace_id = ? AND term = ?",
    )
    .bind(workspace_id.0)
    .bind(term)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_term).transpose()
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<GlossaryTerm>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, term, definition, aliases, created_by, created_at, updated_at
         FROM maidan_glossary_terms WHERE workspace_id = ? ORDER BY term",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_term).collect()
}

/// Remove a term. Returns `true` when a row was deleted.
pub async fn delete(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    term: &str,
) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_glossary_terms WHERE workspace_id = ? AND term = ?")
        .bind(workspace_id.0)
        .bind(term)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

fn row_to_term(row: &sqlx::sqlite::SqliteRow) -> Result<GlossaryTerm, StoreError> {
    let aliases: Vec<String> = serde_json::from_str(row.get::<String, _>("aliases").as_str())?;
    Ok(GlossaryTerm {
        id: row.get::<Uuid, _>("id"),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        term: row.get::<String, _>("term"),
        definition: row.get::<String, _>("definition"),
        aliases,
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
