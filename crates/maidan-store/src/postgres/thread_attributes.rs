//! Task attributes on Postgres: a thread's kind, engine and threat-model
//! flag, stored beside the thread and read back as the defaults when the
//! thread was never described.

use chrono::Utc;
use maidan_types::{
    describe_held, NewAuditEvent, TaskAttributes, TaskEngine, TaskKind, TaskLane, TaskSize,
    TaskTier, ThreadId, UniqueAttributeKey, WorkspaceId, WorkspaceUniqueThreadAttributes,
};
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
    refuse_held_on(conn, thread_id, attributes).await?;
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, kind, engine, threat_model, size, acceptance, linear_issue, tier, lane, author_family, updated_at, repo)
         VALUES ($1, $2, $3, $4, $5, $6, $8, $9, $10, $11, $7, $12)",
    )
    .bind(thread_id.0)
    .bind(attributes.kind.as_str())
    .bind(attributes.engine.map(|engine| engine.as_str()))
    .bind(attributes.threat_model)
    .bind(attributes.size.map(|size| size.as_str()))
    .bind(attributes.acceptance.as_deref())
    .bind(Utc::now())
    .bind(attributes.linear_issue.as_deref())
    .bind(attributes.tier.map(TaskTier::as_str))
    .bind(attributes.lane.map(TaskLane::as_str))
    .bind(attributes.author_family.as_deref())
    .bind(attributes.repo.as_deref())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Refuse `attributes` for `thread_id` when, for one of its workspace's unique
/// key sets, the thread gives every key and another open thread there gives
/// the same values for all of them.
///
/// Race-safe on the caller's transaction: before reading for a holder it
/// takes a transaction-scoped advisory lock on (workspace, key set, values), so
/// two writers of one value run one after the other, and under READ
/// COMMITTED the second one's read sees the first one's committed thread.
/// A unique index cannot say this rule: whether a value is unique depends
/// on the workspace's setting and on the other thread's state, both in other
/// tables. A hash collision only serializes two unrelated writes; the read
/// itself compares the exact value.
pub(crate) async fn refuse_held_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
    attributes: &TaskAttributes,
) -> Result<(), StoreError> {
    if attributes.linear_issue.is_none() && attributes.repo.is_none() {
        return Ok(());
    }
    // A closed, archived or erased thread holds nothing, so setting
    // attributes on one is never refused.
    let row = sqlx::query(
        "SELECT c.workspace_id, w.unique_thread_attributes
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_workspaces w ON w.id = c.workspace_id
         WHERE t.id = $1 AND t.state NOT IN ('closed', 'archived') AND t.tombstoned_at IS NULL",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let workspace_id: uuid::Uuid = row.try_get("workspace_id")?;
    let stored: String = row.try_get("unique_thread_attributes")?;
    // The sets come back sorted, so writers checking several take their
    // locks in one order.
    for set in WorkspaceUniqueThreadAttributes::from_stored(&stored).unique {
        // A thread that lacks a key of the set is not held to it.
        let Some(values) = UniqueAttributeKey::values_of(&set, attributes) else {
            continue;
        };
        let held: Vec<(UniqueAttributeKey, String)> = set
            .iter()
            .copied()
            .zip(values.iter().map(|value| value.to_string()))
            .collect();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 2203))")
            .bind(format!(
                "maidan.unique-attribute:{workspace_id}\n{}",
                describe_held(&held)
            ))
            .execute(&mut *conn)
            .await?;
        let matches: Vec<String> = set
            .iter()
            .enumerate()
            .map(|(n, key)| format!("a.{} = ${}", key.column(), n + 2))
            .collect();
        let sql = format!(
            "SELECT t.id FROM maidan_threads t
             JOIN maidan_channels c ON c.id = t.channel_id
             JOIN maidan_thread_attributes a ON a.thread_id = t.id
             WHERE c.workspace_id = $1 AND {} AND t.id <> ${}
               AND t.state NOT IN ('closed', 'archived') AND t.tombstoned_at IS NULL
             ORDER BY t.created_at, t.id
             LIMIT 1",
            matches.join(" AND "),
            set.len() + 2,
        );
        let mut query = sqlx::query_scalar::<_, uuid::Uuid>(&sql).bind(workspace_id);
        for value in &values {
            query = query.bind(*value);
        }
        let holder = query.bind(thread_id.0).fetch_optional(&mut *conn).await?;
        if let Some(holder) = holder {
            return Err(StoreError::AttributeHeld {
                held,
                holder: ThreadId(holder),
            });
        }
    }
    Ok(())
}

/// The workspace's unique thread attributes.
pub async fn unique_thread_attributes(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceUniqueThreadAttributes, StoreError> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT unique_thread_attributes FROM maidan_workspaces WHERE id = $1")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(WorkspaceUniqueThreadAttributes::from_stored(
        &row.ok_or(StoreError::NotFound)?,
    ))
}

/// Replace the workspace's unique thread attributes, with its audit row in
/// the same transaction. Returns the setting as stored. Threads already
/// open keep their values: the rule refuses the next write that would make
/// a second holder.
pub async fn set_unique_thread_attributes(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    setting: WorkspaceUniqueThreadAttributes,
    audit: NewAuditEvent,
) -> Result<WorkspaceUniqueThreadAttributes, StoreError> {
    let setting = setting.normalized();
    let mut tx = pool.begin().await?;
    let updated =
        sqlx::query("UPDATE maidan_workspaces SET unique_thread_attributes = $2 WHERE id = $1")
            .bind(workspace_id.0)
            .bind(setting.to_stored())
            .execute(&mut *tx)
            .await?;
    if updated.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    super::audit::append_scoped(&mut tx, workspace_id, audit).await?;
    tx.commit().await?;
    Ok(setting)
}

/// Route `thread_id` to its workspace's default engine when it has none,
/// on the caller's connection, inside the create transaction. Runs after
/// any attributes given at creation are stored: an engine named there
/// wins, and the other fields given are kept. With no default, nothing is
/// written.
pub(crate) async fn apply_default_engine_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_attributes (thread_id, engine, updated_at)
         SELECT t.id, w.default_engine, $2
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_workspaces w ON w.id = c.workspace_id
         WHERE t.id = $1 AND w.default_engine IS NOT NULL
         ON CONFLICT (thread_id) DO UPDATE SET engine = excluded.engine
         WHERE maidan_thread_attributes.engine IS NULL",
    )
    .bind(thread_id.0)
    .bind(Utc::now())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The engine a thread created in `workspace_id` with none named is routed
/// to, or `None` when such threads stay unrouted.
pub async fn default_engine(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<TaskEngine>, StoreError> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT default_engine FROM maidan_workspaces WHERE id = $1")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .ok_or(StoreError::NotFound)?
        .and_then(|engine| TaskEngine::parse(&engine)))
}

/// Set or clear the workspace's default engine, with its audit row in the
/// same transaction. Threads created before keep their routing.
pub async fn set_default_engine(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    engine: Option<TaskEngine>,
    audit: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query("UPDATE maidan_workspaces SET default_engine = $2 WHERE id = $1")
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<maidan_types::ReviewChildren, StoreError> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT review_children FROM maidan_workspaces WHERE id = $1")
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
    setting: maidan_types::ReviewChildren,
    audit: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query("UPDATE maidan_workspaces SET review_children = $2 WHERE id = $1")
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

/// Whether `thread_id` is a review child opened through the review family,
/// read in the caller's transaction. Only the server sets `review_family`, so
/// a `kind = review` label alone, set at create or later, exempts nothing
/// from the parent's ordering hold (#1556).
pub(crate) async fn is_review_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM maidan_thread_attributes WHERE thread_id = $1 AND kind = 'review' AND review_family IS NOT NULL",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?
        > 0)
}

/// The thread's attributes, or the defaults when it has no row.
pub async fn get(pool: &PgPool, thread_id: ThreadId) -> Result<TaskAttributes, StoreError> {
    let mut conn = pool.acquire().await?;
    get_on(&mut conn, thread_id).await
}

/// [`get`] on a connection, so a transaction reads its own writes.
pub(crate) async fn get_on(
    conn: &mut sqlx::PgConnection,
    thread_id: ThreadId,
) -> Result<TaskAttributes, StoreError> {
    let row = sqlx::query(
        "SELECT kind, engine, threat_model, size, acceptance, linear_issue, tier, lane, review_family, author_family, repo FROM maidan_thread_attributes WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *conn)
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
        repo: row.try_get("repo")?,
    })
}

/// The attributes a find-by-attribute row carries. The search select
/// left-joins `maidan_thread_attributes` and aliases its columns to their
/// plain names, with `attr_thread_id` marking whether a row exists at all;
/// a thread with no row reads as the defaults, exactly as [`get`] reads it.
pub(crate) fn search_row_attributes(
    row: &sqlx::postgres::PgRow,
) -> Result<TaskAttributes, StoreError> {
    let present: Option<uuid::Uuid> = row.try_get("attr_thread_id")?;
    if present.is_none() {
        return Ok(TaskAttributes::default());
    }
    Ok(TaskAttributes {
        kind: row
            .try_get::<Option<String>, _>("kind")?
            .and_then(|kind| TaskKind::parse(&kind))
            .unwrap_or_default(),
        engine: row
            .try_get::<Option<String>, _>("engine")?
            .and_then(|engine| TaskEngine::parse(&engine)),
        threat_model: row
            .try_get::<Option<bool>, _>("threat_model")?
            .unwrap_or(false),
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
        repo: row.try_get("repo")?,
    })
}

/// Replace the thread's attributes whole. The thread must exist.
pub async fn set(
    pool: &PgPool,
    thread_id: ThreadId,
    attributes: TaskAttributes,
) -> Result<TaskAttributes, StoreError> {
    // Under READ COMMITTED a claim's family check and this set's assignee
    // check could each miss the other's uncommitted write. Both take the
    // thread row lock first (see `review_family::refuse_same_family_in_tx`),
    // so whichever runs second sees the first.
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT 1 FROM maidan_threads WHERE id = $1 FOR UPDATE")
        .bind(thread_id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    // One author family per change: restating it is fine, naming another is
    // refused, and the upsert's COALESCE keeps it even if this read raced.
    if let Some(named) = attributes.author_family.as_deref() {
        let stored: Option<Option<String>> = sqlx::query_scalar(
            "SELECT author_family FROM maidan_thread_attributes WHERE thread_id = $1",
        )
        .bind(thread_id.0)
        .fetch_optional(&mut *tx)
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
        "INSERT INTO maidan_thread_attributes (thread_id, kind, engine, threat_model, size, acceptance, linear_issue, tier, lane, author_family, updated_at, repo)
         VALUES ($1, $2, $3, $4, $5, $6, $8, $9, $10, $11, $7, $12)
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
             repo = excluded.repo,
             updated_at = excluded.updated_at",
    )
    .bind(thread_id.0)
    .bind(attributes.kind.as_str())
    .bind(attributes.engine.map(|engine| engine.as_str()))
    .bind(attributes.threat_model)
    .bind(attributes.size.map(|size| size.as_str()))
    .bind(attributes.acceptance.as_deref())
    .bind(Utc::now())
    .bind(attributes.linear_issue.as_deref())
    .bind(attributes.tier.map(TaskTier::as_str))
    .bind(attributes.lane.map(TaskLane::as_str))
    .bind(attributes.author_family.as_deref())
    .bind(attributes.repo.as_deref())
    .execute(&mut *tx)
    .await?;
    refuse_held_on(&mut tx, thread_id, &attributes).await?;
    // Naming the family after a seat of that family already holds the
    // review would leave it holding work it may not do: every claim-time
    // check ran while the thread was unlabeled. Refuse, and roll back.
    let held_by_family: Option<String> = sqlx::query_scalar(&format!(
        "SELECT {} FROM maidan_threads t WHERE t.id = $1 AND t.assignee_id IS NOT NULL",
        crate::review_family::shared_family("t.assignee_id", "$1")
    ))
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    if let Some(family) = held_by_family {
        return Err(crate::review_family::same_family_refusal(&family));
    }
    tx.commit().await?;
    // The server-set review routing is left as stored and read back, and
    // an author family once stored is kept (see `TaskAttributes`).
    get(pool, thread_id).await
}
