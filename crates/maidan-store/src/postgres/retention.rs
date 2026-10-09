//! Postgres data-retention pruning. Batched deletes (subquery `LIMIT`) so a
//! first sweep over a long-unpruned table doesn't lock it. The event log is
//! partitioned by month, so its old months are dropped whole when they can
//! be; see [`prune_events`].

use chrono::{DateTime, Utc};
use maidan_types::{MessageId, WorkspaceId};
use sqlx::PgPool;
use uuid::Uuid;

use super::partitions;
use crate::error::StoreError;

pub async fn min_delivery_cursor(
    pool: &PgPool,
    advanced_since: DateTime<Utc>,
) -> Result<Option<i64>, StoreError> {
    let row: (Option<i64>,) = sqlx::query_as(
        "SELECT MIN(last_delivered_log_id) FROM maidan_delivery_cursor WHERE updated_at >= $1",
    )
    .bind(advanced_since)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// How long a sweep waits for the event log's lock to drop a partition before
/// it falls back to deleting that partition's rows.
const DROP_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Prune the event log: rows with `id <= max_id` and `occurred_at < cutoff`,
/// outside held workspaces (system events, with no workspace, are never held).
///
/// The log is partitioned by month of `occurred_at` (migration 0162). A month
/// that ends by the cutoff is dropped whole when every row in it would be
/// deleted: nothing above the delivery floor, and nothing of a held
/// workspace. Every other partition that may hold a row before the cutoff (a
/// kept month, the month the cutoff falls in, the pre-partitioning partition
/// and DEFAULT) gets the batched delete, inside that partition only, up to
/// `limit` rows in all. A loop over this ends with exactly the rows the
/// batched delete alone would have left.
///
/// Returns the rows removed, dropped ones included, so the caller's loop runs
/// again after a drop and stops once a call removes fewer than `limit`.
pub async fn prune_events(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    max_id: i64,
    limit: i64,
) -> Result<u64, StoreError> {
    let table = &partitions::EVENTS;
    let mut total = 0u64;
    for part in partitions::list(pool, table).await? {
        if part.ends_by(cutoff) {
            total += drop_events_partition(pool, &part, max_id).await?;
        }
    }
    let mut left = limit;
    for part in partitions::list(pool, table).await? {
        if left <= 0 {
            break;
        }
        if !part.starts_before(cutoff) {
            continue;
        }
        let name = &part.name;
        let res = sqlx::query(&format!(
            "DELETE FROM {name}
             WHERE id IN (
                 SELECT id FROM {name}
                 WHERE id <= $1 AND occurred_at < $2
                   AND (workspace_id IS NULL
                        OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))
                 ORDER BY id ASC
                 LIMIT $3
             )"
        ))
        .bind(max_id)
        .bind(cutoff)
        .bind(left)
        .execute(pool)
        .await?;
        let n = res.rows_affected();
        total += n;
        left = left.saturating_sub(i64::try_from(n).unwrap_or(i64::MAX));
    }
    Ok(total)
}

/// Drop one event-log month that ends by the cutoff, when no row in it is
/// above `max_id` or in a held workspace. Returns the rows it held, or 0 when
/// it is kept (the delete pass then takes its eligible rows) or the table was
/// too busy to lock.
async fn drop_events_partition(
    pool: &PgPool,
    part: &partitions::Partition,
    max_id: i64,
) -> Result<u64, StoreError> {
    let name = &part.name;
    // Counted before the lock, so the lock is held only for the checks, the
    // cascade and the drop. A row added in between is still checked below.
    let rows: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {name}"))
        .fetch_one(pool)
        .await?;
    let Some(mut tx) = partitions::begin_drop(pool, &partitions::EVENTS, DROP_LOCK_WAIT).await?
    else {
        return Ok(0);
    };
    let keeps: bool = sqlx::query_scalar(&format!(
        "SELECT COALESCE((SELECT max(id) FROM {name}) > $1, FALSE)
             OR EXISTS (SELECT 1 FROM maidan_legal_holds h
                        WHERE EXISTS (SELECT 1 FROM {name} e WHERE e.workspace_id = h.workspace_id))"
    ))
    .bind(max_id)
    .fetch_one(&mut *tx)
    .await?;
    if keeps {
        tx.rollback().await?;
        return Ok(0);
    }
    // What the foreign keys' ON DELETE CASCADE did before 0162; a drop fires
    // no delete trigger.
    sqlx::query(&format!(
        "DELETE FROM maidan_outbox WHERE log_id IN (SELECT id FROM {name})"
    ))
    .execute(&mut *tx)
    .await?;
    sqlx::query(&format!(
        "DELETE FROM maidan_federated_ingest WHERE local_event_id IN (SELECT id FROM {name})"
    ))
    .execute(&mut *tx)
    .await?;
    partitions::drop_partition(&mut tx, part).await?;
    tx.commit().await?;
    tracing::info!(partition = %name, rows, "retention: dropped event-log partition");
    Ok(u64::try_from(rows).unwrap_or(0))
}

/// Create the partitions the next months need; see [`partitions::maintain`].
pub async fn maintain_partitions(pool: &PgPool, now: DateTime<Utc>) -> Result<u64, StoreError> {
    partitions::maintain(pool, now).await
}

pub async fn prune_audit(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        // A hold keeps its own workspace's rows, as with events. Instance-level
        // rows (NULL workspace_id) belong to no tenant's hold and still prune.
        "DELETE FROM maidan_audit
         WHERE id IN (
             SELECT id FROM maidan_audit
             WHERE occurred_at < $1
               AND (workspace_id IS NULL
                    OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))
             ORDER BY id ASC
             LIMIT $2
         )",
    )
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Delete up to `limit` read notifications with `created_at < cutoff`.
/// Unread rows stay, and so does any row with a snooze set: lapsing does not
/// make it eligible. A hold keeps its own workspace's rows, as with audit.
pub async fn prune_notifications(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_notifications
         WHERE id IN (
             SELECT id FROM maidan_notifications
             WHERE created_at < $1
               AND read_at IS NOT NULL
               AND snoozed_until IS NULL
               AND (workspace_id IS NULL
                    OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))
             ORDER BY created_at ASC, id ASC
             LIMIT $2
         )",
    )
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// A delivery table's terminal rows: finished work nothing will read again.
/// A row that still asks an operator to act (an egress or mail dead letter,
/// which `MaidanEgressDeadLettered` / `MaidanMailDeadLettered` fire on) is not
/// terminal here: it leaves when it is requeued, not when it is old.
struct TerminalRows {
    table: &'static str,
    /// When the row became terminal (or, for the older tables, was created).
    age: &'static str,
    terminal: &'static str,
    /// The workspace a row belongs to, as SQL over alias `d`, so the instance
    /// sweep can skip a held workspace. NULL (a row whose event or
    /// subscription is gone, or mail with no workspace) belongs to no tenant's
    /// hold, and still prunes.
    hold: &'static str,
    /// The workspace a row belongs to, as SQL over the row, for a workspace's
    /// own retention.
    owner: &'static str,
}

const TERMINAL_ROWS: &[TerminalRows] = &[
    TerminalRows {
        table: "maidan_webhook_deliveries",
        age: "created_at",
        terminal: "(delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)",
        hold: "(SELECT s.workspace_id FROM maidan_webhook_subscriptions s WHERE s.id = d.subscription_id)",
        owner: "(SELECT s.workspace_id FROM maidan_webhook_subscriptions s WHERE s.id = subscription_id)",
    },
    TerminalRows {
        table: "maidan_automation_deliveries",
        age: "created_at",
        terminal: "(delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)",
        hold: "d.workspace_id",
        owner: "workspace_id",
    },
    TerminalRows {
        table: "maidan_outbox",
        age: "published_at",
        terminal: "published_at IS NOT NULL",
        hold: "(SELECT e.workspace_id FROM maidan_events e WHERE e.id = d.log_id)",
        owner: "(SELECT e.workspace_id FROM maidan_events e WHERE e.id = log_id)",
    },
    TerminalRows {
        table: "maidan_egress_outbox",
        age: "updated_at",
        terminal: "status = 'delivered'",
        hold: "d.workspace_id",
        owner: "workspace_id",
    },
    TerminalRows {
        table: "maidan_mail_outbox",
        age: "updated_at",
        terminal: "status = 'delivered'",
        hold: "d.workspace_id",
        owner: "workspace_id",
    },
    TerminalRows {
        table: "maidan_agent_work_dlq",
        age: "failed_at",
        terminal: "TRUE",
        hold: "d.workspace_id",
        owner: "workspace_id",
    },
];

/// Delete up to `limit` terminal rows older than `cutoff` from each delivery
/// table; see [`TERMINAL_ROWS`] for what terminal means per table. A held
/// workspace keeps its rows in every table, including webhook, automation
/// and transactional-outbox rows, whose workspace is not a column of the row.
pub async fn prune_deliveries(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut total = 0u64;
    for rows in TERMINAL_ROWS {
        let TerminalRows {
            table,
            age,
            terminal,
            hold,
            ..
        } = rows;
        // `hold` is SQL over alias `d`. A NULL workspace matches no hold, so
        // those rows still prune, as instance-level audit rows do.
        let sql = format!(
            "DELETE FROM {table}
             WHERE id IN (
                 SELECT d.id FROM {table} d
                 WHERE {age} < $1 AND {terminal}
                   AND NOT EXISTS (
                       SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = {hold}
                   )
                 ORDER BY {age} ASC
                 LIMIT $2
             )"
        );
        let res = sqlx::query(&sql)
            .bind(cutoff)
            .bind(limit)
            .execute(pool)
            .await?;
        total += res.rows_affected();
    }
    Ok(total)
}

/// Delete up to `limit` of one workspace's event-log rows older than `cutoff`,
/// for the workspace's own retention. The delivery-cursor floor is the
/// workspace's own consumers', computed in the statement, and a held workspace
/// loses nothing.
pub async fn prune_workspace_events(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        // The outer `occurred_at` test lets Postgres skip the months after
        // the cutoff; `id` alone would probe every partition's key.
        "DELETE FROM maidan_events
         WHERE occurred_at < $2 AND (id, occurred_at) IN (
             SELECT id, occurred_at FROM maidan_events
             WHERE workspace_id = $1 AND occurred_at < $2
               AND id <= COALESCE(
                   (SELECT MIN(last_delivered_log_id) FROM maidan_delivery_cursor
                    WHERE workspace_id = $1 AND updated_at >= $2),
                   9223372036854775807)
               AND NOT EXISTS (SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = $1)
             ORDER BY id ASC
             LIMIT $3
         )",
    )
    .bind(workspace_id.0)
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// [`prune_deliveries`] for one workspace's rows. A hold keeps every delivery
/// table's rows, as the instance sweep does.
pub async fn prune_workspace_deliveries(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut total = 0u64;
    for rows in TERMINAL_ROWS {
        let TerminalRows {
            table,
            age,
            terminal,
            owner,
            ..
        } = rows;
        let sql = format!(
            "DELETE FROM {table}
             WHERE id IN (
                 SELECT id FROM {table}
                 WHERE {age} < $1 AND {terminal} AND {owner} = $2
                   AND NOT EXISTS (SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = $2)
                 ORDER BY {age} ASC
                 LIMIT $3
             )"
        );
        let res = sqlx::query(&sql)
            .bind(cutoff)
            .bind(workspace_id.0)
            .bind(limit)
            .execute(pool)
            .await?;
        total += res.rows_affected();
    }
    Ok(total)
}

/// Delete up to `limit` of one workspace's messages posted before `cutoff`, as
/// an erasure does: their embeddings, references and content keys go with
/// them (so the sealed copies in the event log can no longer be opened), and
/// they are emptied before they are deleted, as a purge does. A held
/// workspace loses nothing; the hold check locks the workspace row as a
/// purge's does, so a hold placed meanwhile waits for this batch.
pub async fn prune_workspace_messages(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut tx = pool.begin().await?;
    match super::legal_hold::refuse_if_held(&mut tx, workspace_id).await {
        Ok(()) => {}
        Err(StoreError::Conflict(_) | StoreError::NotFound) => return Ok(0),
        Err(err) => return Err(err),
    }
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT m.id FROM maidan_messages m
         INNER JOIN maidan_threads t ON m.thread_id = t.id
         INNER JOIN maidan_channels c ON t.channel_id = c.id
         WHERE c.workspace_id = $1 AND m.posted_at < $2
         ORDER BY m.posted_at ASC
         LIMIT $3",
    )
    .bind(workspace_id.0)
    .bind(cutoff)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    if ids.is_empty() {
        return Ok(0);
    }
    for id in &ids {
        crate::embeddings_purge::purge_message_embeddings_postgres(&mut tx, MessageId(*id)).await?;
        super::content_keys::shred_in_tx(&mut tx, *id).await?;
    }
    sqlx::query(
        "DELETE FROM maidan_references
         WHERE (src_kind = 'message' AND src_id = ANY($1))
            OR (dst_kind = 'message' AND dst_id = ANY($1))",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE maidan_messages
         SET tombstoned_at = COALESCE(tombstoned_at, NOW()), body = '', metadata = '{}', content = NULL
         WHERE id = ANY($1)",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    let res = sqlx::query("DELETE FROM maidan_messages WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected())
}

/// Erase up to `limit` messages posted before `cutoff` across workspaces that
/// are not under a legal hold. Each workspace page reuses
/// [`prune_workspace_messages`], so embeddings, references and content keys go
/// with the messages. A hold placed between the candidate read and the erase
/// drops that workspace for this page.
pub async fn prune_messages(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    if limit <= 0 {
        return Ok(0);
    }
    let workspaces: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT c.workspace_id
         FROM maidan_messages m
         INNER JOIN maidan_threads t ON m.thread_id = t.id
         INNER JOIN maidan_channels c ON t.channel_id = c.id
         WHERE m.posted_at < $1
           AND NOT EXISTS (
               SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = c.workspace_id
           )
         ORDER BY c.workspace_id
         LIMIT $2",
    )
    .bind(cutoff)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut total = 0u64;
    let mut left = limit;
    for id in workspaces {
        if left <= 0 {
            break;
        }
        let n = prune_workspace_messages(pool, WorkspaceId(id), cutoff, left).await?;
        total += n;
        left = left.saturating_sub(i64::try_from(n).unwrap_or(i64::MAX));
    }
    Ok(total)
}
