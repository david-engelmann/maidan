//! Task attributes on Postgres: a thread's kind, engine and threat-model
//! flag, stored beside the thread and read back as the defaults when the
//! thread was never described.

use chrono::Utc;
use maidan_types::{TaskAttributes, TaskEngine, TaskKind, ThreadId};
use sqlx::{PgPool, Row};

use crate::error::StoreError;

/// Store `attributes` for `thread_id` on the caller's connection, inside
/// the create transaction, so the claim queue never sees the thread
/// without them.
pub(crate) async fn insert_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
    attributes: &TaskAttributes,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, kind, engine, threat_model, updated_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(thread_id.0)
    .bind(attributes.kind.as_str())
    .bind(attributes.engine.map(|engine| engine.as_str()))
    .bind(attributes.threat_model)
    .bind(Utc::now())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The thread's attributes, or the defaults when it has no row.
pub async fn get(pool: &PgPool, thread_id: ThreadId) -> Result<TaskAttributes, StoreError> {
    let row = sqlx::query(
        "SELECT kind, engine, threat_model FROM maidan_thread_attributes WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(TaskAttributes::default());
    };
    let kind: String = row.try_get("kind")?;
    let engine: Option<String> = row.try_get("engine")?;
    let threat_model: bool = row.try_get("threat_model")?;
    Ok(TaskAttributes {
        kind: TaskKind::parse(&kind).unwrap_or_default(),
        engine: engine.and_then(|engine| TaskEngine::parse(&engine)),
        threat_model,
    })
}

/// Replace the thread's attributes whole. The thread must exist.
pub async fn set(
    pool: &PgPool,
    thread_id: ThreadId,
    attributes: TaskAttributes,
) -> Result<TaskAttributes, StoreError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM maidan_threads WHERE id = $1)")
            .bind(thread_id.0)
            .fetch_one(pool)
            .await?;
    if !exists {
        return Err(StoreError::NotFound);
    }
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, kind, engine, threat_model, updated_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT(thread_id) DO UPDATE SET
             kind = excluded.kind,
             engine = excluded.engine,
             threat_model = excluded.threat_model,
             updated_at = excluded.updated_at",
    )
    .bind(thread_id.0)
    .bind(attributes.kind.as_str())
    .bind(attributes.engine.map(|engine| engine.as_str()))
    .bind(attributes.threat_model)
    .bind(Utc::now())
    .execute(pool)
    .await?;
    Ok(attributes)
}
