use chrono::{DateTime, Utc};
use maidan_types::{
    NewSlashCommand, SlashCommand, SlashCommandId, SlashCommandWithSecret, SlashHandlerKind,
    WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str =
    "id, workspace_id, name, description, handler_kind, handler_target, secret_ciphertext, enabled, created_at, revoked_at";

pub async fn create(pool: &SqlitePool, new: NewSlashCommand) -> Result<SlashCommand, StoreError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO maidan_slash_commands
            (id, workspace_id, name, description, handler_kind, handler_target, secret_ciphertext)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(&new.description)
    .bind(new.handler_kind.as_str())
    .bind(&new.handler_target)
    .bind(&new.secret_ciphertext)
    .execute(pool)
    .await?;
    get(pool, SlashCommandId(id)).await.map(|row| row.command)
}

pub async fn get(
    pool: &SqlitePool,
    id: SlashCommandId,
) -> Result<SlashCommandWithSecret, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slash_commands WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    let command = row_to_command(&row)?;
    Ok(SlashCommandWithSecret {
        command,
        secret_ciphertext: row.get("secret_ciphertext"),
    })
}

pub async fn get_by_name(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    name: &str,
) -> Result<SlashCommandWithSecret, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_slash_commands
         WHERE workspace_id = ? AND name = ? AND revoked_at IS NULL AND enabled = 1"
    ))
    .bind(workspace_id.0)
    .bind(name)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    let command = row_to_command(&row)?;
    Ok(SlashCommandWithSecret {
        command,
        secret_ciphertext: row.get("secret_ciphertext"),
    })
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<SlashCommand>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_slash_commands
         WHERE workspace_id = ? AND revoked_at IS NULL
         ORDER BY name ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_command).collect()
}

pub async fn revoke(pool: &SqlitePool, id: SlashCommandId) -> Result<SlashCommand, StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_slash_commands
         SET enabled = 0, revoked_at = CURRENT_TIMESTAMP
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(id.0)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    get(pool, id).await.map(|row| row.command)
}

fn row_to_command(row: &sqlx::sqlite::SqliteRow) -> Result<SlashCommand, StoreError> {
    let handler_kind: String = row.get("handler_kind");
    let enabled: i64 = row.get("enabled");
    Ok(SlashCommand {
        id: SlashCommandId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        name: row.get("name"),
        description: row.get("description"),
        handler_kind: SlashHandlerKind::parse(&handler_kind)
            .ok_or_else(|| StoreError::InvalidInput(format!("bad handler_kind: {handler_kind}")))?,
        handler_target: row.get("handler_target"),
        enabled: enabled != 0,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        revoked_at: row.get("revoked_at"),
    })
}
