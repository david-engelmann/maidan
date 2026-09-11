//! Attachable labeled memory-block store (Cluster 373, Wave 2 #21, SQLite twin of
//! pg 0078). A block is a Letta-shaped `{label, description, limit, read_only,
//! value}` workspace object a thread attaches to; `set_value` is a full rewrite
//! (last-writer-wins) refusing a read-only block or an over-limit value.

use chrono::{DateTime, Utc};
use maidan_types::{
    fits_char_limit, MemberId, MemoryBlock, MemoryBlockId, NewMemoryBlock, ThreadId, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str =
    "id, workspace_id, label, description, char_limit, read_only, value, owner_id, created_at, updated_at";
const COLS_B: &str =
    "b.id, b.workspace_id, b.label, b.description, b.char_limit, b.read_only, b.value, b.owner_id, b.created_at, b.updated_at";

fn row_to_block(row: &sqlx::sqlite::SqliteRow) -> MemoryBlock {
    MemoryBlock {
        id: MemoryBlockId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        label: row.get("label"),
        description: row.get::<Option<String>, _>("description"),
        char_limit: row.get::<Option<i64>, _>("char_limit"),
        read_only: row.get::<bool, _>("read_only"),
        value: row.get("value"),
        owner_id: MemberId(row.get::<Uuid, _>("owner_id")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

pub async fn create(pool: &SqlitePool, new: NewMemoryBlock) -> Result<MemoryBlock, StoreError> {
    if !fits_char_limit(&new.value, new.char_limit) {
        return Err(StoreError::InvalidInput(
            "value exceeds the block char limit".into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_memory_blocks
             (id, workspace_id, label, description, char_limit, read_only, value, owner_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, label) DO NOTHING
         RETURNING {COLS}"
    ))
    .bind(MemoryBlockId::new().0)
    .bind(new.workspace_id.0)
    .bind(&new.label)
    .bind(new.description.as_deref())
    .bind(new.char_limit)
    .bind(new.read_only)
    .bind(&new.value)
    .bind(new.owner_id.0)
    .bind(&now)
    .bind(&now)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(row_to_block(&r)),
        None => get_by_label(pool, new.workspace_id, &new.label)
            .await?
            .ok_or(StoreError::NotFound),
    }
}

pub async fn get(pool: &SqlitePool, id: MemoryBlockId) -> Result<Option<MemoryBlock>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_memory_blocks WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_block))
}

pub async fn get_by_label(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    label: &str,
) -> Result<Option<MemoryBlock>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_memory_blocks WHERE workspace_id = ? AND label = ?"
    ))
    .bind(workspace_id.0)
    .bind(label)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_block))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<MemoryBlock>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_memory_blocks WHERE workspace_id = ? ORDER BY label"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_block).collect())
}

/// Full-rewrite the value (last-writer-wins). `NotFound` unknown, `InvalidInput`
/// read-only / over-limit. SQLite serializes writers, so a select-then-update in
/// one tx is race-safe without `FOR UPDATE`.
pub async fn set_value(
    pool: &SqlitePool,
    id: MemoryBlockId,
    value: &str,
) -> Result<MemoryBlock, StoreError> {
    let mut tx = pool.begin().await?;
    let existing = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_memory_blocks WHERE id = ?"
    ))
    .bind(id.0)
    .fetch_optional(&mut *tx)
    .await?;
    let block = existing
        .as_ref()
        .map(row_to_block)
        .ok_or(StoreError::NotFound)?;
    if block.read_only {
        return Err(StoreError::InvalidInput("memory block is read-only".into()));
    }
    if !fits_char_limit(value, block.char_limit) {
        return Err(StoreError::InvalidInput(
            "value exceeds the block char limit".into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "UPDATE maidan_memory_blocks SET value = ?, updated_at = ? WHERE id = ? RETURNING {COLS}"
    ))
    .bind(value)
    .bind(&now)
    .bind(id.0)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row_to_block(&row))
}

pub async fn delete(pool: &SqlitePool, id: MemoryBlockId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_memory_blocks WHERE id = ?")
        .bind(id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn attach(
    pool: &SqlitePool,
    thread_id: ThreadId,
    block_id: MemoryBlockId,
) -> Result<bool, StoreError> {
    let now = Utc::now().to_rfc3339();
    let done = sqlx::query(
        "INSERT INTO maidan_thread_memory_blocks (thread_id, block_id, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT (thread_id, block_id) DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(block_id.0)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn detach(
    pool: &SqlitePool,
    thread_id: ThreadId,
    block_id: MemoryBlockId,
) -> Result<bool, StoreError> {
    let done =
        sqlx::query("DELETE FROM maidan_thread_memory_blocks WHERE thread_id = ? AND block_id = ?")
            .bind(thread_id.0)
            .bind(block_id.0)
            .execute(pool)
            .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn list_for_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<MemoryBlock>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS_B} FROM maidan_memory_blocks b
         JOIN maidan_thread_memory_blocks tmb ON tmb.block_id = b.id
         WHERE tmb.thread_id = ?
         ORDER BY b.label"
    ))
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_block).collect())
}
