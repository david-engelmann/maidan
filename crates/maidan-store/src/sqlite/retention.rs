//! SQLite data-retention pruning. Batched deletes (subquery `LIMIT`) so a first
//! sweep over a long-unpruned table doesn't lock it.

use chrono::{DateTime, Utc};
use maidan_types::{MessageId, WorkspaceId};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::StoreError;

pub async fn min_delivery_cursor(
    pool: &SqlitePool,
    advanced_since: DateTime<Utc>,
) -> Result<Option<i64>, StoreError> {
    // `updated_at` is SQLite `CURRENT_TIMESTAMP` (`YYYY-MM-DD HH:MM:SS`).
    // The cutoff is RFC3339. Text `>=` treats a later time on the cutoff's
    // calendar day as earlier, because ` ` < `T`, and then drops that
    // consumer's floor. `julianday` compares the instants.
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT MIN(last_delivered_log_id) FROM maidan_delivery_cursor WHERE julianday(updated_at) >= julianday(?)",
    )
    .bind(advanced_since)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.0))
}

pub async fn prune_events(
    pool: &SqlitePool,
    cutoff: DateTime<Utc>,
    max_id: i64,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        // Held workspaces' events are exempt (system events with NULL
        // workspace_id are never under a tenant hold, so they still prune).
        "DELETE FROM maidan_events
         WHERE id IN (
             SELECT id FROM maidan_events
             WHERE id <= ? AND occurred_at < ?
               AND (workspace_id IS NULL
                    OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))
             ORDER BY id ASC
             LIMIT ?
         )",
    )
    .bind(max_id)
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn prune_audit(
    pool: &SqlitePool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        // A hold keeps its own workspace's rows, as with events. Instance-level
        // rows (NULL workspace_id) belong to no tenant's hold and still prune.
        "DELETE FROM maidan_audit
         WHERE id IN (
             SELECT id FROM maidan_audit
             WHERE occurred_at < ?
               AND (workspace_id IS NULL
                    OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))
             ORDER BY id ASC
             LIMIT ?
         )",
    )
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Delete up to `limit` read notifications with `created_at` before
/// `cutoff`. Unread rows stay, and so does any row with a snooze set:
/// lapsing does not make it eligible. Times go through `julianday` (the
/// column is RFC 3339 text). A hold keeps its own workspace's rows, as
/// with audit.
pub async fn prune_notifications(
    pool: &SqlitePool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_notifications
         WHERE id IN (
             SELECT id FROM maidan_notifications
             WHERE julianday(created_at) < julianday(?)
               AND read_at IS NOT NULL
               AND snoozed_until IS NULL
               AND (workspace_id IS NULL
                    OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))
             ORDER BY julianday(created_at) ASC, id ASC
             LIMIT ?
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
/// table; see [`TERMINAL_ROWS`] for what terminal means per table. Times are
/// compared through `julianday`, because these tables store them in more than
/// one text format (`CURRENT_TIMESTAMP`'s space, RFC 3339's `T`). A held
/// workspace keeps its rows in every table, including webhook, automation
/// and transactional-outbox rows, whose workspace is not a column of the row.
pub async fn prune_deliveries(
    pool: &SqlitePool,
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
                 WHERE julianday({age}) < julianday(?) AND {terminal}
                   AND NOT EXISTS (
                       SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = {hold}
                   )
                 ORDER BY julianday({age}) ASC
                 LIMIT ?
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

/// One workspace's event-log rows older than `cutoff`; see the Postgres twin.
pub async fn prune_workspace_events(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_events
         WHERE id IN (
             SELECT id FROM maidan_events
             WHERE workspace_id = ?1 AND occurred_at < ?2
               AND id <= COALESCE(
                   (SELECT MIN(last_delivered_log_id) FROM maidan_delivery_cursor
                    WHERE workspace_id = ?1
                      AND julianday(updated_at) >= julianday(?2)),
                   9223372036854775807)
               AND NOT EXISTS (SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = ?1)
             ORDER BY id ASC
             LIMIT ?3
         )",
    )
    .bind(workspace_id.0)
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// [`prune_deliveries`] for one workspace's rows; see the Postgres twin.
/// A hold keeps every table's rows, as the instance sweep does.
pub async fn prune_workspace_deliveries(
    pool: &SqlitePool,
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
                 WHERE julianday({age}) < julianday(?1) AND {terminal} AND {owner} = ?2
                   AND NOT EXISTS (SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = ?2)
                 ORDER BY julianday({age}) ASC
                 LIMIT ?3
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

/// One workspace's messages posted before `cutoff`, erased as the Postgres
/// twin describes. Emptying them first fires the full-text trigger, which
/// only a tombstone does, so their words leave the search index too.
pub async fn prune_workspace_messages(
    pool: &SqlitePool,
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
         WHERE c.workspace_id = ? AND julianday(m.posted_at) < julianday(?)
         ORDER BY julianday(m.posted_at) ASC
         LIMIT ?",
    )
    .bind(workspace_id.0)
    .bind(cutoff)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let now = Utc::now();
    let mut deleted = 0u64;
    for id in ids {
        crate::embeddings_purge::purge_message_embeddings_sqlite(&mut tx, MessageId(id)).await?;
        super::content_keys::shred_in_tx(&mut tx, id).await?;
        sqlx::query(
            "DELETE FROM maidan_references
             WHERE (src_kind = 'message' AND src_id = ?1)
                OR (dst_kind = 'message' AND dst_id = ?1)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE maidan_messages
             SET tombstoned_at = COALESCE(tombstoned_at, ?), body = '', metadata = '{}', content = NULL
             WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        deleted += sqlx::query("DELETE FROM maidan_messages WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;
    Ok(deleted)
}

/// SQLite twin of the Postgres `prune_messages`. `julianday` matches
/// [`prune_workspace_messages`], so a candidate workspace is one that page
/// can actually erase.
pub async fn prune_messages(
    pool: &SqlitePool,
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
         WHERE julianday(m.posted_at) < julianday(?)
           AND NOT EXISTS (
               SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = c.workspace_id
           )
         ORDER BY c.workspace_id
         LIMIT ?",
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
