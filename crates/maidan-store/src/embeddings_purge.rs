//! Purge embedding rows across per-model tables for a workspace.

use maidan_types::WorkspaceId;
use sqlx::{PgPool, SqlitePool};

use crate::error::StoreError;

fn assert_registry_table(table: &str) -> Result<(), StoreError> {
    if !table.starts_with("maidan_emb_") {
        return Err(StoreError::InvalidInput(format!(
            "invalid embedding table name in registry: {table}"
        )));
    }
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StoreError::InvalidInput(format!(
            "invalid embedding table name in registry: {table}"
        )));
    }
    Ok(())
}

pub async fn purge_workspace_embeddings_postgres(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<i64, StoreError> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(pool)
        .await?;
    let mut total = 0i64;
    for table in tables {
        assert_registry_table(&table)?;
        let sql = format!(
            "DELETE FROM {table} WHERE message_id IN (
               SELECT m.id FROM maidan_messages m
               INNER JOIN maidan_threads t ON m.thread_id = t.id
               INNER JOIN maidan_channels c ON t.channel_id = c.id
               WHERE c.workspace_id = $1
             )"
        );
        let result = sqlx::query(&sql).bind(workspace_id.0).execute(pool).await?;
        total += i64::try_from(result.rows_affected()).unwrap_or(0);
    }
    Ok(total)
}

pub async fn purge_workspace_embeddings_sqlite(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<i64, StoreError> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(pool)
        .await?;
    let mut total = 0i64;
    for table in tables {
        assert_registry_table(&table)?;
        let sql = format!(
            "DELETE FROM {table} WHERE message_id IN (
               SELECT m.id FROM maidan_messages m
               INNER JOIN maidan_threads t ON m.thread_id = t.id
               INNER JOIN maidan_channels c ON t.channel_id = c.id
               WHERE c.workspace_id = ?
             )"
        );
        let result = sqlx::query(&sql).bind(workspace_id.0).execute(pool).await?;
        total += i64::try_from(result.rows_affected()).unwrap_or(0);
    }
    Ok(total)
}
