//! SQLite-backed one-time OAuth authorization codes (Cluster 104).
//!
//! The TTL comparison binds `Utc::now()` rather than a SQL `strftime` so both
//! sides use sqlx's own `DateTime<Utc>` text encoding — keeping the lexical
//! comparison consistent regardless of format.

use chrono::{DateTime, Utc};
use maidan_types::{AppId, NewOAuthCode, OAuthCode, WorkspaceId};
use sqlx::SqlitePool;

use crate::error::StoreError;

/// `(app_id, workspace_id, redirect_uri, code_challenge, expires_at)`.
type CodeRow = (
    uuid::Uuid,
    uuid::Uuid,
    String,
    Option<String>,
    DateTime<Utc>,
);

pub async fn insert(pool: &SqlitePool, new: NewOAuthCode) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_oauth_codes WHERE expires_at <= ?")
        .bind(Utc::now())
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO maidan_oauth_codes
            (code_hash, app_id, workspace_id, redirect_uri, code_challenge, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)",
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

pub async fn consume(pool: &SqlitePool, code_hash: &str) -> Result<Option<OAuthCode>, StoreError> {
    let row: Option<CodeRow> = sqlx::query_as(
        "DELETE FROM maidan_oauth_codes
             WHERE code_hash = ? AND expires_at > ?
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
