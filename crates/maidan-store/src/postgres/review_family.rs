//! Second-family review routing on Postgres (S4).

use chrono::Utc;
use maidan_types::{
    provider_family, review_family_for, Event, MemberId, TaskAttributes, TaskKind, ThreadId,
    WorkspaceId,
};
use sqlx::Row;
use uuid::Uuid;

use super::{events, thread_attributes, threads};
use crate::error::StoreError;

/// When change `thread_id` enters review, spawn its review child, routed to a
/// provider family other than its author's, unless it is not a change or a
/// live review child already exists. The author's family is read from the
/// latest usage record on the change; when none is named there is nothing to
/// route away from, and no child is spawned.
pub(crate) async fn spawn_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
    actor_id: MemberId,
) -> Result<(), StoreError> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM maidan_thread_attributes WHERE thread_id = $1")
            .bind(thread_id.0)
            .fetch_optional(&mut **tx)
            .await?;
    if kind.is_some_and(|kind| kind != TaskKind::Change.as_str()) {
        return Ok(());
    }
    let live_child: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maidan_threads t
         JOIN maidan_thread_attributes a ON a.thread_id = t.id
         WHERE t.parent_thread_id = $1 AND a.kind = 'review'
           AND t.tombstoned_at IS NULL AND t.state NOT IN ('closed', 'archived')",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    if live_child > 0 {
        return Ok(());
    }
    let usage = sqlx::query(
        "SELECT provider, model FROM maidan_usage_ledger
         WHERE thread_id = $1 ORDER BY accepted_at DESC LIMIT 1",
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
        return Ok(());
    };
    let review = review_family_for(Some(author));
    let place = sqlx::query(
        "SELECT t.channel_id, ch.workspace_id FROM maidan_threads t
         JOIN maidan_channels ch ON ch.id = t.channel_id WHERE t.id = $1",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let channel_id: Uuid = place.try_get("channel_id")?;
    let workspace_id: Uuid = place.try_get("workspace_id")?;
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_threads (id, channel_id, parent_thread_id, title, description, created_at, updated_at, created_by)
         VALUES ($1, $2, $3, $4, NULL, $5, $6, $7)
         RETURNING id, channel_id, parent_thread_id, title, description, state, created_at, updated_at, tombstoned_at, assignee_id, assignment_expires_at, claim_lease_id, work_started_at, owner_id, created_by",
    )
    .bind(Uuid::now_v7())
    .bind(channel_id)
    .bind(thread_id.0)
    .bind("Second-family review")
    .bind(now)
    .bind(now)
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
        "UPDATE maidan_thread_attributes SET review_family = $1, author_family = $2 WHERE thread_id = $3",
    )
    .bind(review)
    .bind(author)
    .bind(thread.id.0)
    .execute(&mut **tx)
    .await?;
    thread_attributes::apply_default_engine_on(tx, thread.id).await?;
    events::append_in_tx(
        tx,
        &Event::ThreadCreated {
            occurred_at: now,
            workspace_id: WorkspaceId(workspace_id),
            channel_id: thread.channel_id,
            thread,
        },
    )
    .await?;
    Ok(())
}
