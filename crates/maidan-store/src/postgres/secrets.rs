//! Named-secret store (Cluster 371, Wave 2 #19): the `maidan_secrets` table. The
//! value is stored AEAD-encrypted (the route layer holds the key); metadata reads
//! never select the ciphertext, and `get_ciphertext` is the only path that does.
//! See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, NewSecret, Secret, SecretId, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

const META_COLS: &str = "id, workspace_id, name, created_by, created_at, updated_at";

fn row_to_secret(row: &sqlx::postgres::PgRow) -> Secret {
    Secret {
        id: SecretId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        name: row.get("name"),
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

/// Create or rotate a secret. Re-creating an existing name replaces the value
/// (`updated_at` bumps) — the rotation path. Returns metadata only.
pub async fn create(pool: &PgPool, new: NewSecret) -> Result<Secret, StoreError> {
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_secrets (id, workspace_id, name, value_ciphertext, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
         ON CONFLICT (workspace_id, name) DO UPDATE SET
             value_ciphertext = excluded.value_ciphertext,
             updated_at = NOW()
         RETURNING {META_COLS}"
    ))
    .bind(Uuid::new_v4())
    .bind(new.workspace_id.0)
    .bind(&new.name)
    .bind(&new.value_ciphertext)
    .bind(new.created_by.0)
    .fetch_one(pool)
    .await?;
    Ok(row_to_secret(&row))
}

/// The encrypted value for a named secret, if it exists — the only read that
/// touches the ciphertext (the resolve path decrypts it).
pub async fn get_ciphertext(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    name: &str,
) -> Result<Option<String>, StoreError> {
    let row = sqlx::query(
        "SELECT value_ciphertext FROM maidan_secrets WHERE workspace_id = $1 AND name = $2",
    )
    .bind(workspace_id.0)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get::<String, _>("value_ciphertext")))
}

pub async fn list(pool: &PgPool, workspace_id: WorkspaceId) -> Result<Vec<Secret>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {META_COLS} FROM maidan_secrets WHERE workspace_id = $1 ORDER BY name"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_secret).collect())
}

pub async fn delete(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    name: &str,
) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_secrets WHERE workspace_id = $1 AND name = $2")
        .bind(workspace_id.0)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
