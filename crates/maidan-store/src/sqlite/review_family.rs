//! Second-family review routing on SQLite (S4); see the Postgres twin.

use chrono::Utc;
use maidan_types::{
    provider_family, review_family_for, Event, MemberId, StoredEvent, TaskAttributes, TaskKind,
    ThreadId, WorkspaceId,
};
use sqlx::Row;
use uuid::Uuid;

use super::{events, thread_attributes, threads};
use crate::error::StoreError;

/// When change `thread_id` enters review, spawn its review child, routed to a
/// provider family other than its author's, unless it is not a change or a
/// live review child already exists. The author's family is read from the
/// latest usage record the change's claimant reported on it; when none is
/// named there is nothing to route away from, and no child is spawned.
pub(crate) async fn spawn_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    actor_id: MemberId,
) -> Result<Vec<StoredEvent>, StoreError> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM maidan_thread_attributes WHERE thread_id = ?")
            .bind(thread_id.0)
            .fetch_optional(&mut **tx)
            .await?;
    if kind.is_some_and(|kind| kind != TaskKind::Change.as_str()) {
        return Ok(Vec::new());
    }
    // A workspace that turned review children off spawns none.
    let setting: String = sqlx::query_scalar(
        "SELECT w.review_children FROM maidan_threads t
         JOIN maidan_channels ch ON ch.id = t.channel_id
         JOIN maidan_workspaces w ON w.id = ch.workspace_id
         WHERE t.id = ?",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    if setting == maidan_types::ReviewChildren::Off.as_str() {
        return Ok(Vec::new());
    }
    let live_child: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maidan_threads t
         JOIN maidan_thread_attributes a ON a.thread_id = t.id
         WHERE t.parent_thread_id = ? AND a.kind = 'review'
           AND t.tombstoned_at IS NULL AND t.state NOT IN ('closed', 'archived')",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    if live_child > 0 {
        return Ok(Vec::new());
    }
    // The author is the change's claimant (else its creator); only that
    // seat's own usage records name its family, latest first, ties broken by
    // record id so the pick is deterministic.
    let usage = sqlx::query(
        "SELECT ul.provider, ul.model FROM maidan_usage_ledger ul
         JOIN maidan_threads t ON t.id = ul.thread_id
         JOIN maidan_channels ch ON ch.id = t.channel_id
         WHERE ul.thread_id = ? AND ul.workspace_id = ch.workspace_id
           AND ul.reporter_id = COALESCE(t.assignee_id, t.created_by)
         ORDER BY ul.accepted_at DESC, ul.usage_report_id DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    let author = match &usage {
        Some(row) => provider_family(
            row.try_get::<Option<String>, _>("provider")?.as_deref(),
            &row.try_get::<String, _>("model")?,
        ),
        None => None,
    };
    // Routing needs the author's family; with none named nothing is spawned.
    let Some(author) = author else {
        return Ok(Vec::new());
    };
    let review = review_family_for(Some(author));
    let place = sqlx::query(
        "SELECT t.channel_id, ch.workspace_id FROM maidan_threads t
         JOIN maidan_channels ch ON ch.id = t.channel_id WHERE t.id = ?",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let channel_id: Uuid = place.try_get("channel_id")?;
    let workspace_id: Uuid = place.try_get("workspace_id")?;
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, description, created_at, updated_at, created_by)
         VALUES (?, ?, ?, ?, NULL, ?, ?, ?)
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by",
    )
    .bind(Uuid::now_v7())
    .bind(channel_id)
    .bind(thread_id.0)
    .bind("Second-family review")
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(actor_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let thread = threads::row_to_thread(&row)?;
    let attributes = TaskAttributes {
        kind: TaskKind::Review,
        ..TaskAttributes::default()
    };
    thread_attributes::insert_on(tx, thread.id, &attributes).await?;
    sqlx::query(
        "UPDATE maidan_thread_attributes SET review_family = ?, author_family = ? WHERE thread_id = ?",
    )
    .bind(review)
    .bind(author)
    .bind(thread.id.0)
    .execute(&mut **tx)
    .await?;
    thread_attributes::apply_default_engine_on(tx, thread.id).await?;
    let (child_id, channel) = (thread.id, thread.channel_id);
    let created = events::append_in_tx(
        tx,
        &Event::ThreadCreated {
            occurred_at: now,
            workspace_id: WorkspaceId(workspace_id),
            channel_id: channel,
            thread,
        },
    )
    .await?;
    // The routing lives in the attributes row; say it on the log too, so
    // replay and projections see it (#1572).
    let attributes = thread_attributes::get_on(tx, child_id).await?;
    let routed = events::append_in_tx(
        tx,
        &Event::ThreadAttributesSet {
            occurred_at: now,
            workspace_id: WorkspaceId(workspace_id),
            channel_id: channel,
            thread_id: child_id,
            attributes,
            author_family: Some(author.to_owned()),
            actor_id,
        },
    )
    .await?;
    Ok(vec![created, routed])
}

/// Refuse handing review `thread_id` to `member_id` when the member's family
/// wrote the change it reviews (see [`crate::review_family::shared_family`]),
/// with an error naming the family rather than a silent miss.
pub(crate) async fn refuse_same_family_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    let family: Option<String> = sqlx::query_scalar(&format!(
        "SELECT {}",
        crate::review_family::shared_family("?1", "?2")
    ))
    .bind(member_id.0)
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    match family {
        Some(family) => Err(crate::review_family::same_family_refusal(&family)),
        None => Ok(()),
    }
}
