use chrono::{DateTime, Utc};
use maidan_types::{ApiToken, ApiTokenId, MemberId, NewApiToken, WorkspaceId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewApiToken) -> Result<ApiToken, StoreError> {
    let id = Uuid::new_v4();
    let capabilities = serde_json::to_string(&new.capabilities)?;
    let row = sqlx::query(
        "INSERT INTO maidan_api_tokens
            (id, workspace_id, member_id, token_hash, label, capabilities, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, workspace_id, member_id, token_hash, label, capabilities,
                   created_at, expires_at, revoked_at",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.member_id.0)
    .bind(&new.token_hash)
    .bind(new.label.as_deref())
    .bind(&capabilities)
    .bind(new.expires_at)
    .fetch_one(pool)
    .await
    .map_err(map_token_err)?;
    row_to_token(&row)
}

pub async fn get_active_by_hash(pool: &PgPool, token_hash: &str) -> Result<ApiToken, StoreError> {
    let row = sqlx::query(
        "SELECT id, workspace_id, member_id, token_hash, label, capabilities,
                created_at, expires_at, revoked_at
         FROM maidan_api_tokens
         WHERE token_hash = $1
           AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > NOW())",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_token(&row)
}

pub async fn revoke(pool: &PgPool, id: ApiTokenId) -> Result<ApiToken, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(
        "UPDATE maidan_api_tokens
         SET revoked_at = $2
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING id, workspace_id, member_id, token_hash, label, capabilities,
                   created_at, expires_at, revoked_at",
    )
    .bind(id.0)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_token(&row)
}

fn map_token_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict("token hash already exists".into());
        }
    }
    StoreError::Database(err)
}

fn row_to_token(row: &sqlx::postgres::PgRow) -> Result<ApiToken, StoreError> {
    let capabilities_json: String = row.get("capabilities");
    let capabilities: Vec<String> = serde_json::from_str(&capabilities_json).map_err(|e| {
        StoreError::InvalidInput(format!("invalid capabilities JSON in database: {e}"))
    })?;
    Ok(ApiToken {
        id: ApiTokenId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        token_hash: row.get("token_hash"),
        label: row.get("label"),
        capabilities,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        expires_at: row.get::<Option<DateTime<Utc>>, _>("expires_at"),
        revoked_at: row.get::<Option<DateTime<Utc>>, _>("revoked_at"),
    })
}
