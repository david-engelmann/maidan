use chrono::{DateTime, Utc};
use maidan_types::{
    NewSlashCommand, SlashCommand, SlashCommandId, SlashCommandWithSecret, SlashHandlerKind,
    WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str =
    "id, workspace_id, name, description, handler_kind, handler_target, secret_ciphertext, enabled, created_at, revoked_at";

pub async fn create(pool: &PgPool, new: NewSlashCommand) -> Result<SlashCommand, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_slash_commands
            (id, workspace_id, name, description, handler_kind, handler_target, secret_ciphertext)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {COLS}"
    ))
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(&new.description)
    .bind(new.handler_kind.as_str())
    .bind(&new.handler_target)
    .bind(&new.secret_ciphertext)
    .fetch_one(pool)
    .await?;
    row_to_command(&row)
}

pub async fn get(pool: &PgPool, id: SlashCommandId) -> Result<SlashCommandWithSecret, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slash_commands WHERE id = $1"
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
    name: &str,
) -> Result<SlashCommandWithSecret, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_slash_commands
         WHERE workspace_id = $1 AND name = $2 AND revoked_at IS NULL AND enabled = TRUE"
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<SlashCommand>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_slash_commands
         WHERE workspace_id = $1 AND revoked_at IS NULL
         ORDER BY name ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_command).collect()
}

pub async fn revoke(pool: &PgPool, id: SlashCommandId) -> Result<SlashCommand, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_slash_commands
         SET enabled = FALSE, revoked_at = NOW()
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING {COLS}"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_command(&row)
}

fn row_to_command(row: &sqlx::postgres::PgRow) -> Result<SlashCommand, StoreError> {
    let handler_kind: String = row.get("handler_kind");
    Ok(SlashCommand {
        id: SlashCommandId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        name: row.get("name"),
        description: row.get("description"),
        handler_kind: SlashHandlerKind::parse(&handler_kind)
            .ok_or_else(|| StoreError::InvalidInput(format!("bad handler_kind: {handler_kind}")))?,
        handler_target: row.get("handler_target"),
        enabled: row.get("enabled"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        revoked_at: row.get("revoked_at"),
    })
}
