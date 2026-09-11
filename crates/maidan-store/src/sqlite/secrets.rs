//! Named-secret store (Cluster 371, SQLite twin of pg 0076). The value is stored
//! AEAD-encrypted; metadata reads never select the ciphertext.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, NewSecret, Secret, SecretId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const META_COLS: &str = "id, workspace_id, name, created_by, created_at, updated_at";

fn row_to_secret(row: &sqlx::sqlite::SqliteRow) -> Secret {
    Secret {
        id: SecretId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        name: row.get("name"),
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

pub async fn create(pool: &SqlitePool, new: NewSecret) -> Result<Secret, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_secrets (id, workspace_id, name, value_ciphertext, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, name) DO UPDATE SET
             value_ciphertext = excluded.value_ciphertext,
             updated_at = excluded.updated_at
         RETURNING {META_COLS}"
    ))
    .bind(SecretId::new().0)
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(&new.value_ciphertext)
    .bind(new.created_by.0)
    .bind(&now)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_secret(&row))
}

pub async fn get_ciphertext(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    name: &str,
) -> Result<Option<String>, StoreError> {
    let row = sqlx::query(
        "SELECT value_ciphertext FROM maidan_secrets WHERE workspace_id = ? AND name = ?",
    )
    .bind(workspace_id.0)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get::<String, _>("value_ciphertext")))
}

pub async fn list(pool: &SqlitePool, workspace_id: WorkspaceId) -> Result<Vec<Secret>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {META_COLS} FROM maidan_secrets WHERE workspace_id = ? ORDER BY name"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_secret).collect())
}

pub async fn delete(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    name: &str,
) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_secrets WHERE workspace_id = ? AND name = ?")
        .bind(workspace_id.0)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
