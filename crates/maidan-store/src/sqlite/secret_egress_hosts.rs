//! The hosts each workspace trusts to receive its secret values: the
//! SecretBroker substitutes a `secret://` ref on egress only to a host listed
//! here for the sending workspace. SQLite twin of the Postgres module —
//! timestamps are store-bound rfc3339 text.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{
    normalize_secret_egress_host, NewSecretEgressHost, SecretEgressHost, WorkspaceId,
};

const COLS: &str = "workspace_id, host, created_at";

/// List a host. Idempotent: a host already listed keeps its original entry.
pub async fn allow(
    pool: &SqlitePool,
    new: NewSecretEgressHost,
) -> Result<SecretEgressHost, StoreError> {
    let mut conn = pool.acquire().await?;
    allow_on(&mut conn, new).await
}

pub(crate) async fn allow_on(
    conn: &mut sqlx::SqliteConnection,
    new: NewSecretEgressHost,
) -> Result<SecretEgressHost, StoreError> {
    let host = normalize_secret_egress_host(&new.host)
        .map_err(|why| StoreError::InvalidInput(why.to_string()))?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_secret_egress_hosts (workspace_id, host, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT (workspace_id, host) DO UPDATE
           SET host = excluded.host
         RETURNING {COLS}"
    ))
    .bind(new.workspace_id.0)
    .bind(&host)
    .bind(Utc::now().to_rfc3339())
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_host(&row))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<SecretEgressHost>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_secret_egress_hosts
         WHERE workspace_id = ?
         ORDER BY host ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_host).collect())
}

/// Remove a host. Scoped to the workspace; `false` when it was not listed.
pub async fn revoke(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    host: &str,
) -> Result<bool, StoreError> {
    let mut conn = pool.acquire().await?;
    revoke_on(&mut conn, workspace_id, host).await
}

pub(crate) async fn revoke_on(
    conn: &mut sqlx::SqliteConnection,
    workspace_id: WorkspaceId,
    host: &str,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_secret_egress_hosts WHERE workspace_id = ? AND host = ?")
            .bind(workspace_id.0)
            .bind(host.to_ascii_lowercase())
            .execute(&mut *conn)
            .await?;
    Ok(res.rows_affected() > 0)
}

/// The check the broker makes before it substitutes. `host` is the egress
/// URL's parsed host.
pub async fn is_allowed(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    host: &str,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT 1 AS present FROM maidan_secret_egress_hosts
         WHERE workspace_id = ? AND host = ?",
    )
    .bind(workspace_id.0)
    .bind(host.to_ascii_lowercase())
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

fn row_to_host(row: &sqlx::sqlite::SqliteRow) -> SecretEgressHost {
    SecretEgressHost {
        workspace_id: WorkspaceId(row.get("workspace_id")),
        host: row.get("host"),
        created_at: row.get("created_at"),
    }
}
