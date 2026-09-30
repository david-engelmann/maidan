//! Postgres data-retention pruning. Batched deletes (subquery `LIMIT`) so a
//! first sweep over a long-unpruned table doesn't lock it.

use chrono::{DateTime, Utc};
use maidan_types::{MessageId, WorkspaceId};
use sqlx::PgPool;
use uuid::Uuid;

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

pub async fn prune_events(
    pool: &PgPool,
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
             WHERE id <= $1 AND occurred_at < $2
               AND (workspace_id IS NULL
                    OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))
             ORDER BY id ASC
             LIMIT $3
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

/// A delivery table's terminal rows: finished work nothing will read again.
/// A row that still asks an operator to act (an egress or mail dead letter,
/// which `MaidanEgressDeadLettered` / `MaidanMailDeadLettered` fire on) is not
/// terminal here: it leaves when it is requeued, not when it is old.
struct TerminalRows {
    table: &'static str,
    /// When the row became terminal (or, for the older tables, was created).
    age: &'static str,
    terminal: &'static str,
    /// Whether rows carry a `workspace_id` a legal hold exempts, like events.
    held: bool,
    /// The workspace a row belongs to, as SQL over the row, for a workspace's
    /// own retention.
    owner: &'static str,
}

const TERMINAL_ROWS: &[TerminalRows] = &[
    TerminalRows {
        table: "maidan_webhook_deliveries",
        age: "created_at",
        terminal: "(delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)",
        held: false,
        owner: "(SELECT s.workspace_id FROM maidan_webhook_subscriptions s WHERE s.id = subscription_id)",
    },
    TerminalRows {
        table: "maidan_automation_deliveries",
        age: "created_at",
        terminal: "(delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)",
        held: false,
        owner: "workspace_id",
    },
    TerminalRows {
        table: "maidan_outbox",
        age: "published_at",
        terminal: "published_at IS NOT NULL",
        held: false,
        owner: "(SELECT e.workspace_id FROM maidan_events e WHERE e.id = log_id)",
    },
    TerminalRows {
        table: "maidan_egress_outbox",
        age: "updated_at",
        terminal: "status = 'delivered'",
        held: true,
        owner: "workspace_id",
    },
    TerminalRows {
        table: "maidan_mail_outbox",
        age: "updated_at",
        terminal: "status = 'delivered'",
        held: true,
        owner: "workspace_id",
    },
    TerminalRows {
        table: "maidan_agent_work_dlq",
        age: "failed_at",
        terminal: "TRUE",
        held: true,
        owner: "workspace_id",
    },
];

/// Delete up to `limit` terminal rows older than `cutoff` from each delivery
/// table; see [`TERMINAL_ROWS`] for what terminal means per table.
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
            held,
            ..
        } = rows;
        let unheld = if *held {
            "AND (workspace_id IS NULL
                  OR workspace_id NOT IN (SELECT workspace_id FROM maidan_legal_holds))"
        } else {
            ""
        };
        let sql = format!(
            "DELETE FROM {table}
             WHERE id IN (
                 SELECT id FROM {table}
                 WHERE {age} < $1 AND {terminal} {unheld}
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
        "DELETE FROM maidan_events
         WHERE id IN (
             SELECT id FROM maidan_events
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
/// table's rows here, not only those the instance sweep exempts.
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
