//! Postgres-backed one-time OAuth authorization codes (Cluster 104).

use chrono::{DateTime, Utc};
use maidan_types::{AppId, NewOAuthCode, OAuthCode, WorkspaceId};
use sqlx::PgPool;

use crate::error::StoreError;

/// `(app_id, workspace_id, redirect_uri, code_challenge, expires_at)`.
type CodeRow = (
    uuid::Uuid,
    uuid::Uuid,
    String,
    Option<String>,
    DateTime<Utc>,
);

pub async fn insert(pool: &PgPool, new: NewOAuthCode) -> Result<(), StoreError> {
    // Opportunistically reclaim expired rows on each mint (a rare operation).
    sqlx::query("DELETE FROM maidan_oauth_codes WHERE expires_at <= NOW()")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO maidan_oauth_codes
            (code_hash, app_id, workspace_id, redirect_uri, code_challenge, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(new.code_hash)
    .bind(new.app_id.0)
    .bind(new.workspace_id.0)
    .bind(new.redirect_uri)
    .bind(new.code_challenge)
    .bind(new.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn consume(pool: &PgPool, code_hash: &str) -> Result<Option<OAuthCode>, StoreError> {
    let row: Option<CodeRow> = sqlx::query_as(
        "DELETE FROM maidan_oauth_codes
             WHERE code_hash = $1 AND expires_at > $2
             RETURNING app_id, workspace_id, redirect_uri, code_challenge, expires_at",
    )
    .bind(code_hash)
    .bind(Utc::now())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(app_id, workspace_id, redirect_uri, code_challenge, expires_at)| OAuthCode {
            app_id: AppId(app_id),
            workspace_id: WorkspaceId(workspace_id),
            redirect_uri,
            code_challenge,
            expires_at,
        },
    ))
}
