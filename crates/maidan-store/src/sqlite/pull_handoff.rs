//! A superseded thread hands its open pull requests to the thread that
//! supersedes it; see the Postgres twin.

use chrono::Utc;
use maidan_types::{ContentKeyring, MemberId, NewAuditEvent, StoredEvent, ThreadId};
use sqlx::Row;
use uuid::Uuid;

use crate::cancel::{handed_away_body, handed_to_body, handoff_metadata};
use crate::error::StoreError;

/// One pull request a handoff moved.
pub(crate) struct HandedPull {
    pub repo: String,
    pub pull_number: i64,
}

/// Move every pull request a close of `from` would withdraw onto `to`, in
/// the close's transaction; see the Postgres twin.
pub(crate) async fn hand_off_pulls_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    keys: &ContentKeyring,
    from: ThreadId,
    to: ThreadId,
    actor: MemberId,
    audit: &NewAuditEvent,
) -> Result<(Vec<HandedPull>, Vec<StoredEvent>), StoreError> {
    let pulls = sqlx::query(
        "SELECT p.workspace_id, p.repo, p.pull_number
         FROM maidan_change_pulls p
         WHERE p.thread_id = ?1 AND p.closed_at IS NULL AND NOT p.watch_only
           AND EXISTS (
               SELECT 1 FROM maidan_change_rounds r
               WHERE r.workspace_id = p.workspace_id AND r.repo = p.repo
                 AND r.pull_number = p.pull_number)
         ORDER BY p.repo, p.pull_number",
    )
    .bind(from.0)
    .fetch_all(&mut **tx)
    .await?;
    if pulls.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    // The close holds SQLite's one write lock, so a cancel of the successor
    // either committed before this read or runs after the move.
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM maidan_threads WHERE id = ?1 AND tombstoned_at IS NULL",
    )
    .bind(to.0)
    .fetch_optional(&mut **tx)
    .await?;
    let first = format!(
        "{}#{}",
        pulls[0].get::<String, _>("repo"),
        pulls[0].get::<i64, _>("pull_number")
    );
    match state.as_deref() {
        Some("open" | "in_review") => {}
        other => {
            return Err(StoreError::Conflict(format!(
                "{} is {}, so it cannot take pull request {first}; name an open successor, or close without duplicate_of to withdraw it",
                to.0,
                other.unwrap_or("gone")
            )));
        }
    }
    let mut handed = Vec::with_capacity(pulls.len());
    let mut stored = Vec::with_capacity(pulls.len() * 2);
    for pull in &pulls {
        let workspace_id: Uuid = pull.get("workspace_id");
        let repo: String = pull.get("repo");
        let pull_number: i64 = pull.get("pull_number");
        let held: Option<i64> = sqlx::query_scalar(
            "SELECT pull_number FROM maidan_change_pulls
             WHERE thread_id = ?1 AND workspace_id = ?2 AND repo = ?3
               AND closed_at IS NULL AND NOT watch_only
             ORDER BY pull_number LIMIT 1",
        )
        .bind(to.0)
        .bind(workspace_id)
        .bind(&repo)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(held) = held {
            return Err(StoreError::Conflict(format!(
                "{} already tracks open pull request {repo}#{held}, so it cannot also take {repo}#{pull_number}; close or withdraw one of them first, or close without duplicate_of to withdraw {repo}#{pull_number}",
                to.0
            )));
        }
        sqlx::query(
            "UPDATE maidan_change_pulls SET thread_id = ?4, handed_at = ?5
             WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3",
        )
        .bind(workspace_id)
        .bind(&repo)
        .bind(pull_number)
        .bind(to.0)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut **tx)
        .await?;
        // Deploys are filed by thread as well; they follow the pull request
        // so its deploy notices reach the thread that now answers for it.
        sqlx::query(
            "UPDATE maidan_deploys SET thread_id = ?5
             WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3 AND thread_id = ?4",
        )
        .bind(workspace_id)
        .bind(&repo)
        .bind(pull_number)
        .bind(from.0)
        .bind(to.0)
        .execute(&mut **tx)
        .await?;
        let metadata = handoff_metadata(&repo, pull_number, from, to);
        super::audit::append_scoped(
            tx,
            crate::audit_scope::AuditOwner::Thread(to),
            NewAuditEvent {
                scope: audit.scope,
                actor_id: audit.actor_id,
                action: "thread.pull_handed_off".into(),
                target_kind: Some("github_pull".into()),
                target_id: None,
                metadata: metadata.clone(),
            },
        )
        .await?;
        stored.push(
            super::plans::post_notice_in_tx(
                tx,
                keys,
                from,
                actor,
                handed_away_body(&repo, pull_number, to),
                metadata.clone(),
            )
            .await?,
        );
        stored.push(
            super::plans::post_notice_in_tx(
                tx,
                keys,
                to,
                actor,
                handed_to_body(&repo, pull_number, from),
                metadata,
            )
            .await?,
        );
        handed.push(HandedPull { repo, pull_number });
    }
    Ok((handed, stored))
}
