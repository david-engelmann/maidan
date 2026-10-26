//! Task attributes on SQLite: a thread's kind, engine and threat-model
//! flag, stored beside the thread and read back as the defaults when the
//! thread was never described.

use chrono::Utc;
use maidan_types::{
    NewAuditEvent, TaskAttributes, TaskEngine, TaskKind, TaskLane, TaskSize, TaskTier, ThreadId,
    WorkspaceId,
};
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;

/// Store `attributes` for `thread_id` on the caller's connection, inside
/// the create transaction, so the claim queue never sees the thread
/// without them.
pub(crate) async fn insert_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
    attributes: &TaskAttributes,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, kind, engine, threat_model, size, acceptance, linear_issue, tier, lane, author_family, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(thread_id.0)
    .bind(attributes.kind.as_str())
    .bind(attributes.engine.map(|engine| engine.as_str()))
    .bind(attributes.threat_model)
    .bind(attributes.size.map(|size| size.as_str()))
    .bind(attributes.acceptance.as_deref())
    .bind(attributes.linear_issue.as_deref())
    .bind(attributes.tier.map(TaskTier::as_str))
    .bind(attributes.lane.map(TaskLane::as_str))
    .bind(attributes.author_family.as_deref())
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Route `thread_id` to its workspace's default engine when it has none;
/// see the Postgres twin.
pub(crate) async fn apply_default_engine_on(
    conn: &mut sqlx::SqliteConnection,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, engine, updated_at)
         SELECT t.id, w.default_engine, ?2
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_workspaces w ON w.id = c.workspace_id
         WHERE t.id = ?1 AND w.default_engine IS NOT NULL
         ON CONFLICT (thread_id) DO UPDATE SET engine = excluded.engine
         WHERE maidan_thread_attributes.engine IS NULL",
    )
    .bind(thread_id.0)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The workspace's default engine; see the Postgres twin.
pub async fn default_engine(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Option<TaskEngine>, StoreError> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT default_engine FROM maidan_workspaces WHERE id = ?1")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .ok_or(StoreError::NotFound)?
        .and_then(|engine| TaskEngine::parse(&engine)))
}

/// Set or clear the workspace's default engine, audited in the same
/// transaction.
pub async fn set_default_engine(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    engine: Option<TaskEngine>,
    audit: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query("UPDATE maidan_workspaces SET default_engine = ?2 WHERE id = ?1")
        .bind(workspace_id.0)
        .bind(engine.map(TaskEngine::as_str))
        .execute(&mut *tx)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    super::audit::append_scoped(&mut tx, workspace_id, audit).await?;
    tx.commit().await?;
    Ok(())
}

/// The workspace's review-children setting.
pub async fn review_children(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<maidan_types::ReviewChildren, StoreError> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT review_children FROM maidan_workspaces WHERE id = ?1")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?;
    let stored = row.ok_or(StoreError::NotFound)?;
    // The column's CHECK admits only the two values.
    Ok(maidan_types::ReviewChildren::parse(&stored).unwrap_or_default())
}

/// Set the workspace's review-children setting, with its audit row in the
/// same transaction. Changes already in review keep their children.
pub async fn set_review_children(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    setting: maidan_types::ReviewChildren,
    audit: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query("UPDATE maidan_workspaces SET review_children = ?2 WHERE id = ?1")
        .bind(workspace_id.0)
        .bind(setting.as_str())
        .execute(&mut *tx)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    super::audit::append_scoped(&mut tx, workspace_id, audit).await?;
    tx.commit().await?;
    Ok(())
}

/// Whether `thread_id` is a review task, read in the caller's transaction.
pub(crate) async fn is_review_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM maidan_thread_attributes WHERE thread_id = ? AND kind = 'review'",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?
        > 0)
}

/// The thread's attributes, or the defaults when it has no row.
pub async fn get(pool: &SqlitePool, thread_id: ThreadId) -> Result<TaskAttributes, StoreError> {
    let row = sqlx::query(
        "SELECT kind, engine, threat_model, size, acceptance, linear_issue, tier, lane, review_family, author_family FROM maidan_thread_attributes WHERE thread_id = ?",
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
        size: row
            .try_get::<Option<String>, _>("size")?
            .and_then(|size| TaskSize::parse(&size)),
        acceptance: row.try_get("acceptance")?,
        linear_issue: row.try_get("linear_issue")?,
        tier: row
            .try_get::<Option<String>, _>("tier")?
            .and_then(|tier| TaskTier::parse(&tier)),
        lane: row
            .try_get::<Option<String>, _>("lane")?
            .and_then(|lane| TaskLane::parse(&lane)),
        review_family: row.try_get("review_family")?,
        author_family: row.try_get("author_family")?,
    })
}

/// Replace the thread's attributes whole. The thread must exist.
pub async fn set(
    pool: &SqlitePool,
    thread_id: ThreadId,
    attributes: TaskAttributes,
) -> Result<TaskAttributes, StoreError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM maidan_threads WHERE id = ?)")
            .bind(thread_id.0)
            .fetch_one(pool)
            .await?;
    if !exists {
        return Err(StoreError::NotFound);
    }
    // One author family per change: restating it is fine, naming another is
    // refused, and the upsert's COALESCE keeps it even if this read raced.
    if let Some(named) = attributes.author_family.as_deref() {
        let stored: Option<Option<String>> = sqlx::query_scalar(
            "SELECT author_family FROM maidan_thread_attributes WHERE thread_id = ?",
        )
        .bind(thread_id.0)
        .fetch_optional(pool)
        .await?;
        if let Some(Some(stored)) = stored
            && stored != named
        {
            return Err(StoreError::Conflict(format!(
                "author_family is {stored} and stays {stored}: a review's author family is set once"
            )));
        }
    }
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, kind, engine, threat_model, size, acceptance, linear_issue, tier, lane, author_family, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(thread_id) DO UPDATE SET
             kind = excluded.kind,
             engine = excluded.engine,
             threat_model = excluded.threat_model,
             size = excluded.size,
             acceptance = excluded.acceptance,
             linear_issue = excluded.linear_issue,
             tier = excluded.tier,
             lane = excluded.lane,
             author_family = COALESCE(maidan_thread_attributes.author_family, excluded.author_family),
             updated_at = excluded.updated_at",
    )
    .bind(thread_id.0)
    .bind(attributes.kind.as_str())
    .bind(attributes.engine.map(|engine| engine.as_str()))
    .bind(attributes.threat_model)
    .bind(attributes.size.map(|size| size.as_str()))
    .bind(attributes.acceptance.as_deref())
    .bind(attributes.linear_issue.as_deref())
    .bind(attributes.tier.map(TaskTier::as_str))
    .bind(attributes.lane.map(TaskLane::as_str))
    .bind(attributes.author_family.as_deref())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    // The server-set review routing is left as stored and read back, and
    // an author family once stored is kept (see `TaskAttributes`).
    get(pool, thread_id).await
}
