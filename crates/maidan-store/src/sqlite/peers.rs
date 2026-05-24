use chrono::{DateTime, Utc};
use maidan_types::{NewPeer, Peer, PeerId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const PEER_COLS: &str =
    "id, workspace_id, remote_workspace_id, name, base_url, token_hash, outbound_secret_ciphertext, \
                          enabled, last_synced_event_id, created_at, updated_at";

pub async fn create(pool: &SqlitePool, new: NewPeer) -> Result<Peer, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_peers
            (id, workspace_id, remote_workspace_id, name, base_url, token_hash, outbound_secret_ciphertext, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING {PEER_COLS}"
    ))
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(new.remote_workspace_id.0)
    .bind(&new.name)
    .bind(&new.base_url)
    .bind(&new.token_hash)
    .bind(&new.outbound_secret_ciphertext)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await
    .map_err(map_peer_err)?;
    row_to_peer(&row)
}

pub async fn get(pool: &SqlitePool, id: PeerId) -> Result<Peer, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {PEER_COLS} FROM maidan_peers WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_peer(&row)
}

pub async fn get_by_token_hash(pool: &SqlitePool, token_hash: &str) -> Result<Peer, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {PEER_COLS}
         FROM maidan_peers
         WHERE token_hash = ? AND enabled = 1"
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_peer(&row)
}

pub async fn list(pool: &SqlitePool, workspace_id: WorkspaceId) -> Result<Vec<Peer>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {PEER_COLS}
         FROM maidan_peers
         WHERE workspace_id = ?
         ORDER BY name ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_peer).collect()
}

pub async fn list_enabled(pool: &SqlitePool) -> Result<Vec<Peer>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {PEER_COLS}
         FROM maidan_peers
         WHERE enabled = 1
         ORDER BY name ASC"
    ))
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_peer).collect()
}

pub async fn update_cursor(
    pool: &SqlitePool,
    id: PeerId,
    last_synced_event_id: i64,
) -> Result<Peer, StoreError> {
    let now = Utc::now();
    let row = sqlx::query(&format!(
        "UPDATE maidan_peers
         SET last_synced_event_id = ?, updated_at = ?
         WHERE id = ?
         RETURNING {PEER_COLS}"
    ))
    .bind(last_synced_event_id)
    .bind(now)
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_peer(&row)
}

pub async fn delete(pool: &SqlitePool, id: PeerId) -> Result<(), StoreError> {
    let result = sqlx::query("DELETE FROM maidan_peers WHERE id = ?")
        .bind(id.0)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn ingest_exists(
    pool: &SqlitePool,
    peer_id: PeerId,
    remote_event_id: i64,
) -> Result<bool, StoreError> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM maidan_federated_ingest
         WHERE peer_id = ? AND remote_event_id = ?
         LIMIT 1",
    )
    .bind(peer_id.0)
    .bind(remote_event_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

pub async fn try_record_ingest(
    pool: &SqlitePool,
    peer_id: PeerId,
    remote_event_id: i64,
    local_event_id: i64,
) -> Result<bool, StoreError> {
    let now = Utc::now();
    let result = sqlx::query(
        "INSERT INTO maidan_federated_ingest (peer_id, remote_event_id, local_event_id, ingested_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (peer_id, remote_event_id) DO NOTHING",
    )
    .bind(peer_id.0)
    .bind(remote_event_id)
    .bind(local_event_id)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn is_federated_local_event(
    pool: &SqlitePool,
    local_event_id: i64,
) -> Result<bool, StoreError> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM maidan_federated_ingest WHERE local_event_id = ? LIMIT 1")
            .bind(local_event_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

fn map_peer_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict("peer token hash already exists".into());
        }
    }
    StoreError::Database(err)
}

fn row_to_peer(row: &sqlx::sqlite::SqliteRow) -> Result<Peer, StoreError> {
    Ok(Peer {
        id: PeerId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        remote_workspace_id: WorkspaceId(row.get::<Uuid, _>("remote_workspace_id")),
        name: row.get("name"),
        base_url: row.get("base_url"),
        token_hash: row.get("token_hash"),
        outbound_secret_ciphertext: row.get("outbound_secret_ciphertext"),
        enabled: row.get::<i64, _>("enabled") != 0,
        last_synced_event_id: row.get("last_synced_event_id"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
