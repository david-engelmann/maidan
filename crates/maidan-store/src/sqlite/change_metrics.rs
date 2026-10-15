//! Swarm ramp metrics on SQLite; see the Postgres twin.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use maidan_types::{ChangePull, ChangeRound, NewChangeRound, ThreadId, WorkspaceId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Record a delivered round, and the pull request the first time. A retried
/// delivery of the same commit adds nothing.
pub async fn record_round(pool: &SqlitePool, new: NewChangeRound) -> Result<(), StoreError> {
    let repo = new.repo.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_change_pulls (repo, pull_number, workspace_id, thread_id, opened_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (repo, pull_number) DO NOTHING",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.workspace_id.0)
    .bind(new.thread_id.0)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO maidan_change_rounds
             (repo, pull_number, commit_sha, round, base_sha, base_behind, diff_lines, delivered_at)
         SELECT ?1, ?2, ?3, COALESCE(MAX(round), 0) + 1, ?4, ?5, ?6, ?7
         FROM maidan_change_rounds WHERE repo = ?1 AND pull_number = ?2
         ON CONFLICT (repo, pull_number, commit_sha) DO NOTHING",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.commit_sha.to_ascii_lowercase())
    .bind(&new.base_sha)
    .bind(new.base_behind)
    .bind(new.diff_lines)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Stamp when a tracked pull request was made ready (the first time only).
pub async fn mark_ready(
    pool: &SqlitePool,
    repo: &str,
    pull_number: i64,
    at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls SET ready_at = COALESCE(ready_at, ?3)
         WHERE repo = ?1 AND pull_number = ?2",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Record that a tracked pull request closed, merged or not.
pub async fn close(
    pool: &SqlitePool,
    repo: &str,
    pull_number: i64,
    merged_at: Option<DateTime<Utc>>,
    merged_head_sha: Option<&str>,
    closed_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_change_pulls
         SET merged_at = ?3, merged_head_sha = ?4, closed_at = ?5
         WHERE repo = ?1 AND pull_number = ?2 AND closed_at IS NULL",
    )
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .bind(merged_at.map(|at| at.to_rfc3339()))
    .bind(merged_head_sha.map(str::to_ascii_lowercase))
    .bind(closed_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// The tracked pull requests in `workspace_id` not yet seen closed, oldest
/// first, at most `limit`.
pub async fn list_open(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<(String, i64)>, StoreError> {
    let rows = sqlx::query(
        "SELECT repo, pull_number FROM maidan_change_pulls
         WHERE workspace_id = ?1 AND closed_at IS NULL
         ORDER BY opened_at LIMIT ?2",
    )
    .bind(workspace_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("repo"), row.get("pull_number")))
        .collect())
}

/// The tracked pull requests in `workspace_id`, in `repo` and opened since
/// `since` when given, oldest first, each with its rounds and its thread's
/// model cost.
pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: Option<&str>,
    since: Option<DateTime<Utc>>,
    engine: Option<maidan_types::TaskEngine>,
    kind: Option<maidan_types::TaskKind>,
) -> Result<Vec<ChangePull>, StoreError> {
    let repo = repo.map(str::to_ascii_lowercase);
    let since = since.map(|at| at.to_rfc3339());
    let pulls = sqlx::query(
        "SELECT p.repo, p.pull_number, p.thread_id, p.opened_at, p.ready_at, p.merged_at, p.merged_head_sha, p.closed_at,
                a.kind AS task_kind, a.engine AS task_engine, a.threat_model AS threat_model
         FROM maidan_change_pulls p
         LEFT JOIN maidan_thread_attributes a ON a.thread_id = p.thread_id
         WHERE p.workspace_id = ?1
           AND (?2 IS NULL OR p.repo = ?2)
           AND (?3 IS NULL OR p.opened_at >= ?3)
           AND (?4 IS NULL OR a.engine = ?4)
           AND (?5 IS NULL OR COALESCE(a.kind, 'change') = ?5)
         ORDER BY p.opened_at, p.repo, p.pull_number
         LIMIT 1000",
    )
    .bind(workspace_id.0)
    .bind(&repo)
    .bind(&since)
    .bind(engine.map(|engine| engine.as_str()))
    .bind(kind.map(|kind| kind.as_str()))
    .fetch_all(pool)
    .await?;
    let rounds = sqlx::query(
        "SELECT r.repo, r.pull_number, r.commit_sha, r.round, r.base_behind, r.diff_lines, r.delivered_at
         FROM maidan_change_rounds r
         JOIN maidan_change_pulls p ON p.repo = r.repo AND p.pull_number = r.pull_number
         WHERE p.workspace_id = ?1
         ORDER BY r.repo, r.pull_number, r.round",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let mut by_pull: HashMap<(String, i64), Vec<ChangeRound>> = HashMap::new();
    for row in &rounds {
        by_pull
            .entry((row.get("repo"), row.get("pull_number")))
            .or_default()
            .push(ChangeRound {
                round: row.get("round"),
                commit_sha: row.get("commit_sha"),
                base_behind: row.get("base_behind"),
                diff_lines: row.get("diff_lines"),
                delivered_at: row.get::<DateTime<Utc>, _>("delivered_at"),
            });
    }
    let costs = sqlx::query(
        "SELECT thread_id, COALESCE(SUM(usd_micros), 0) AS cost
         FROM maidan_usage_ledger WHERE workspace_id = ?1
         GROUP BY thread_id",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let cost: HashMap<Uuid, i64> = costs
        .iter()
        .map(|row| (row.get("thread_id"), row.get("cost")))
        .collect();
    Ok(pulls
        .iter()
        .map(|row| pull_from(row, &mut by_pull, &cost))
        .collect())
}

fn pull_from(
    row: &sqlx::sqlite::SqliteRow,
    rounds: &mut HashMap<(String, i64), Vec<ChangeRound>>,
    cost: &HashMap<Uuid, i64>,
) -> ChangePull {
    let repo: String = row.get("repo");
    let pull_number: i64 = row.get("pull_number");
    let thread: Uuid = row.get("thread_id");
    ChangePull {
        thread_id: ThreadId(thread),
        rounds: rounds
            .remove(&(repo.clone(), pull_number))
            .unwrap_or_default(),
        repo,
        pull_number,
        opened_at: row.get::<DateTime<Utc>, _>("opened_at"),
        ready_at: row.get::<Option<DateTime<Utc>>, _>("ready_at"),
        merged_at: row.get::<Option<DateTime<Utc>>, _>("merged_at"),
        merged_head_sha: row.get("merged_head_sha"),
        closed_at: row.get::<Option<DateTime<Utc>>, _>("closed_at"),
        cost_usd_micros: cost.get(&thread).copied().unwrap_or(0),
        attributes: maidan_types::TaskAttributes {
            kind: row
                .try_get::<Option<String>, _>("task_kind")
                .ok()
                .flatten()
                .and_then(|kind| maidan_types::TaskKind::parse(&kind))
                .unwrap_or_default(),
            engine: row
                .try_get::<Option<String>, _>("task_engine")
                .ok()
                .flatten()
                .and_then(|engine| maidan_types::TaskEngine::parse(&engine)),
            threat_model: row
                .try_get::<Option<bool>, _>("threat_model")
                .ok()
                .flatten()
                .unwrap_or(false),
        },
    }
}
