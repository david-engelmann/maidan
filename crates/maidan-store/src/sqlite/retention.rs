//! SQLite data-retention pruning. Batched deletes (subquery `LIMIT`) so a first
//! sweep over a long-unpruned table doesn't lock it.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::error::StoreError;

pub async fn min_delivery_cursor(
    pool: &SqlitePool,
    advanced_since: DateTime<Utc>,
) -> Result<Option<i64>, StoreError> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT MIN(last_delivered_log_id) FROM maidan_delivery_cursor WHERE updated_at >= ?",
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
        // Maidan_audit is not workspace-tagged, so a legal hold freezes audit
        // pruning entirely while any hold is active.
        "DELETE FROM maidan_audit
         WHERE id IN (
             SELECT id FROM maidan_audit
             WHERE occurred_at < ?
               AND NOT EXISTS (SELECT 1 FROM maidan_legal_holds)
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
}

const TERMINAL_ROWS: &[TerminalRows] = &[
    TerminalRows {
        table: "maidan_webhook_deliveries",
        age: "created_at",
        terminal: "(delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)",
        held: false,
    },
    TerminalRows {
        table: "maidan_automation_deliveries",
        age: "created_at",
        terminal: "(delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)",
        held: false,
    },
    TerminalRows {
        table: "maidan_outbox",
        age: "published_at",
        terminal: "published_at IS NOT NULL",
        held: false,
    },
    TerminalRows {
        table: "maidan_egress_outbox",
        age: "updated_at",
        terminal: "status = 'delivered'",
        held: true,
    },
    TerminalRows {
        table: "maidan_mail_outbox",
        age: "updated_at",
        terminal: "status = 'delivered'",
        held: true,
    },
    TerminalRows {
        table: "maidan_agent_work_dlq",
        age: "failed_at",
        terminal: "TRUE",
        held: true,
    },
];

/// Delete up to `limit` terminal rows older than `cutoff` from each delivery
/// table; see [`TERMINAL_ROWS`] for what terminal means per table. Times are
/// compared through `julianday`, because these tables store them in more than
/// one text format (`CURRENT_TIMESTAMP`'s space, RFC 3339's `T`).
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
            held,
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
                 WHERE julianday({age}) < julianday(?) AND {terminal} {unheld}
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
