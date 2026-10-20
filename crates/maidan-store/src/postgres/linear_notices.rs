//! Claim and release comments on a task's Linear issue; the notices are
//! described in [`maidan_types::linear_notice`].

use chrono::Utc;
use maidan_types::{
    LinearNoticeDue, LinearNoticeKind, NewAuditEvent, NewEgressOutbox, TaskEngine, ThreadId,
    WorkspaceId,
};
use sqlx::{PgPool, Row};

use crate::error::StoreError;

/// The workspaces that have turned claim comments on, at most `limit`.
pub async fn list_enabled(pool: &PgPool, limit: i64) -> Result<Vec<WorkspaceId>, StoreError> {
    let rows = sqlx::query(
        "SELECT id FROM maidan_workspaces WHERE linear_claim_comments = TRUE
         ORDER BY id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|row| WorkspaceId(row.get("id"))).collect())
}

/// Whether `workspace_id` has turned claim comments on.
pub async fn enabled(pool: &PgPool, workspace_id: WorkspaceId) -> Result<bool, StoreError> {
    let on: Option<bool> =
        sqlx::query_scalar("SELECT linear_claim_comments FROM maidan_workspaces WHERE id = $1")
            .bind(workspace_id.0)
            .fetch_optional(pool)
            .await?;
    on.ok_or(StoreError::NotFound)
}

/// Turn claim comments on or off, with its audit row in the same
/// transaction.
pub async fn set_enabled(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    on: bool,
    audit: NewAuditEvent,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let updated =
        sqlx::query("UPDATE maidan_workspaces SET linear_claim_comments = $2 WHERE id = $1")
            .bind(workspace_id.0)
            .bind(on)
            .execute(&mut *tx)
            .await?;
    if updated.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    super::audit::append_counted(&mut tx, audit).await?;
    tx.commit().await?;
    Ok(())
}

/// The notices due in `workspace_id`, at most `limit` of each kind: a claim
/// for a claimed task whose attributes name a Linear issue and whose claim
/// was not said yet, and a release for a task whose claim was said that has
/// since closed with no merged pull request.
pub async fn list_due(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<LinearNoticeDue>, StoreError> {
    let claims = sqlx::query(
        "SELECT t.id AS thread_id, t.title, a.linear_issue, a.engine, m.handle AS holder
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_thread_attributes a ON a.thread_id = t.id
         LEFT JOIN maidan_members m ON m.id = t.assignee_id
         WHERE c.workspace_id = $1 AND a.linear_issue IS NOT NULL AND t.assignee_id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM maidan_linear_notices n WHERE n.thread_id = t.id AND n.kind = 'claim')
         ORDER BY t.id LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let releases = sqlx::query(
        "SELECT t.id AS thread_id, t.title, a.linear_issue, a.engine, NULL AS holder,
                h.rounds AS handoff_rounds
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_thread_attributes a ON a.thread_id = t.id
         LEFT JOIN maidan_thread_handoffs h ON h.thread_id = t.id
         WHERE c.workspace_id = $1 AND a.linear_issue IS NOT NULL
           AND (h.thread_id IS NOT NULL OR (t.state IN ('closed', 'archived')
               AND NOT EXISTS (
                   SELECT 1 FROM maidan_change_pulls p
                   WHERE p.thread_id = t.id AND p.merged_at IS NOT NULL)))
           AND EXISTS (
               SELECT 1 FROM maidan_linear_notices n WHERE n.thread_id = t.id AND n.kind = 'claim')
           AND NOT EXISTS (
               SELECT 1 FROM maidan_linear_notices n WHERE n.thread_id = t.id AND n.kind = 'release')
         ORDER BY t.id LIMIT $2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let due = |row: &sqlx::postgres::PgRow, kind: LinearNoticeKind| LinearNoticeDue {
        workspace_id,
        thread_id: ThreadId(row.get("thread_id")),
        kind,
        issue: row.get("linear_issue"),
        title: row.get("title"),
        // A handoff moved the task to `session`; its release still names
        // the engine that held it, and only Pi tasks are handed off.
        engine: if row
            .try_get::<Option<i64>, _>("handoff_rounds")
            .ok()
            .flatten()
            .is_some()
        {
            Some(TaskEngine::Pi)
        } else {
            row.get::<Option<String>, _>("engine")
                .and_then(|engine| TaskEngine::parse(&engine))
        },
        holder: row.get("holder"),
        reason: (kind == LinearNoticeKind::Release).then(|| {
            match row
                .try_get::<Option<i64>, _>("handoff_rounds")
                .ok()
                .flatten()
            {
                Some(rounds) => format!("handed off to a session after {rounds} rounds"),
                None => "ended without merging".to_string(),
            }
        }),
    };
    Ok(claims
        .iter()
        .map(|row| due(row, LinearNoticeKind::Claim))
        .chain(
            releases
                .iter()
                .map(|row| due(row, LinearNoticeKind::Release)),
        )
        .collect())
}

/// Record that `due` was handled, with `outcome` (`queued`, or why not),
/// and queue `send` with it in the same transaction. `false`, and nothing
/// queued, when it was handled already: each notice is said once.
pub async fn record(
    pool: &PgPool,
    due: &LinearNoticeDue,
    outcome: &str,
    send: Option<NewEgressOutbox>,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO maidan_linear_notices (thread_id, kind, workspace_id, issue, outcome, created_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (thread_id, kind) DO NOTHING",
    )
    .bind(due.thread_id.0)
    .bind(due.kind.as_str())
    .bind(due.workspace_id.0)
    .bind(&due.issue)
    .bind(outcome)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(false);
    }
    if let Some(new) = send {
        enqueue_on(&mut tx, new).await?;
    }
    tx.commit().await?;
    Ok(true)
}

async fn enqueue_on(conn: &mut sqlx::PgConnection, new: NewEgressOutbox) -> Result<(), StoreError> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_egress_outbox
           (id, workspace_id, thread_id, source_log_id, surface, selector, body, kind,
            status, attempts, next_attempt_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', 0, $9, $9, $9)
         ON CONFLICT (source_log_id, surface, selector) DO NOTHING",
    )
    .bind(maidan_types::EgressOutboxId::new().0)
    .bind(new.workspace_id.0)
    .bind(new.thread_id.0)
    .bind(new.source_log_id)
    .bind(new.target.surface().as_str())
    .bind(new.target.selector())
    .bind(&new.body)
    .bind(new.kind.as_str())
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
