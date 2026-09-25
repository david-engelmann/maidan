//! Purge embedding rows across per-model tables for a workspace.

use maidan_types::WorkspaceId;

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
    conn: &mut sqlx::PgConnection,
    workspace_id: WorkspaceId,
) -> Result<i64, StoreError> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(&mut *conn)
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
        let result = sqlx::query(&sql)
            .bind(workspace_id.0)
            .execute(&mut *conn)
            .await?;
        total += i64::try_from(result.rows_affected()).unwrap_or(0);
    }
    Ok(total)
}

pub async fn purge_workspace_embeddings_sqlite(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
) -> Result<i64, StoreError> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(&mut *conn)
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
        let result = sqlx::query(&sql)
            .bind(workspace_id.0)
            .execute(&mut *conn)
            .await?;
        total += i64::try_from(result.rows_affected()).unwrap_or(0);
    }
    Ok(total)
}

/// Delete one message's embeddings in every model table. A withdrawn message
/// is not searchable, and its vectors are derived from words it withdrew.
pub(crate) async fn purge_message_embeddings_postgres(
    conn: &mut sqlx::PgConnection,
    message_id: maidan_types::MessageId,
) -> Result<(), StoreError> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(&mut *conn)
        .await?;
    for table in tables {
        assert_registry_table(&table)?;
        sqlx::query(&format!("DELETE FROM {table} WHERE message_id = $1"))
            .bind(message_id.0)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// SQLite twin of [`purge_message_embeddings_postgres`].
pub(crate) async fn purge_message_embeddings_sqlite(
    conn: &mut sqlx::SqliteConnection,
    message_id: maidan_types::MessageId,
) -> Result<(), StoreError> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(&mut *conn)
        .await?;
    for table in tables {
        assert_registry_table(&table)?;
        sqlx::query(&format!("DELETE FROM {table} WHERE message_id = ?"))
            .bind(message_id.0)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
