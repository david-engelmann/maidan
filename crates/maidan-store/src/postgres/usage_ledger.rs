use maidan_types::{
    ClaimLeaseId, MemberId, NewUsageLedgerEntry, PayerStamp, PriceSnapshot, ThreadBudget, ThreadId,
    TokenUsage, UsageLedgerEntry, WorkspaceId,
};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::StoreError;

pub async fn get(
    pool: &PgPool,
    usage_report_id: uuid::Uuid,
) -> Result<Option<UsageLedgerEntry>, StoreError> {
    let row = sqlx::query("SELECT * FROM maidan_usage_ledger WHERE usage_report_id = $1")
        .bind(usage_report_id)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(row_to_entry).transpose()
}

pub async fn list_for_thread(
    pool: &PgPool,
    thread_id: ThreadId,
    limit: i64,
) -> Result<Vec<UsageLedgerEntry>, StoreError> {
    let limit = limit.clamp(1, 100);
    let rows = sqlx::query(
        "SELECT * FROM maidan_usage_ledger
         WHERE thread_id = $1 AND budget IS NOT NULL
         ORDER BY accepted_at DESC, usage_report_id DESC LIMIT $2",
    )
    .bind(thread_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_entry).collect()
}

/// Reserve the global idempotency key inside the caller's transaction.
/// `true` means this transaction owns the new row; `false` means a completed
/// exact/conflicting retry must be read and compared after the unique-key wait.
pub(super) async fn reserve_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    new: &NewUsageLedgerEntry,
    workspace_id: WorkspaceId,
) -> Result<bool, StoreError> {
    let result = sqlx::query(
        "INSERT INTO maidan_usage_ledger (
            usage_report_id, workspace_id, thread_id, reporter_id,
            claim_lease_id, model, input_tokens, output_tokens,
            cache_read_tokens, cache_write_tokens, input_price, output_price,
            cache_read_price, cache_write_price, usd_micros, turns
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
         ON CONFLICT (usage_report_id) DO NOTHING",
    )
    .bind(new.usage_report_id)
    .bind(workspace_id.0)
    .bind(new.thread_id.0)
    .bind(new.reporter.0)
    .bind(new.claim_lease_id.0)
    .bind(new.model.trim())
    .bind(new.tokens.input)
    .bind(new.tokens.output)
    .bind(new.tokens.cache_read)
    .bind(new.tokens.cache_write)
    .bind(new.price_snapshot.input_usd_micros_per_million)
    .bind(new.price_snapshot.output_usd_micros_per_million)
    .bind(new.price_snapshot.cache_read_usd_micros_per_million)
    .bind(new.price_snapshot.cache_write_usd_micros_per_million)
    .bind(new.usd_micros)
    .bind(new.turns)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub(super) async fn get_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    usage_report_id: uuid::Uuid,
) -> Result<Option<UsageLedgerEntry>, StoreError> {
    let row = sqlx::query("SELECT * FROM maidan_usage_ledger WHERE usage_report_id = $1")
        .bind(usage_report_id)
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref().map(row_to_entry).transpose()
}

pub(super) async fn finish_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    usage_report_id: uuid::Uuid,
    budget: &ThreadBudget,
    stopped: bool,
    reason: Option<&str>,
    usage_event_id: i64,
    claim_failed_event_id: Option<i64>,
) -> Result<UsageLedgerEntry, StoreError> {
    let budget = serde_json::to_value(budget)?;
    let row = sqlx::query(
        "UPDATE maidan_usage_ledger SET
            budget = $2, stopped = $3, reason = $4, usage_event_id = $5,
            claim_failed_event_id = $6
         WHERE usage_report_id = $1 RETURNING *",
    )
    .bind(usage_report_id)
    .bind(budget)
    .bind(stopped)
    .bind(reason)
    .bind(usage_event_id)
    .bind(claim_failed_event_id)
    .fetch_one(&mut **tx)
    .await?;
    row_to_entry(&row)
}

fn row_to_entry(row: &sqlx::postgres::PgRow) -> Result<UsageLedgerEntry, StoreError> {
    let budget = row
        .get::<Option<serde_json::Value>, _>("budget")
        .ok_or_else(|| StoreError::Conflict("usage report is still pending".into()))?;
    Ok(UsageLedgerEntry {
        usage_report_id: row.get("usage_report_id"),
        thread_id: ThreadId(row.get("thread_id")),
        stamp: PayerStamp {
            payer: WorkspaceId(row.get("workspace_id")),
            reporter: MemberId(row.get("reporter_id")),
            claim_lease_id: ClaimLeaseId(row.get("claim_lease_id")),
            model: row.get("model"),
            tokens: TokenUsage {
                input: row.get("input_tokens"),
                output: row.get("output_tokens"),
                cache_read: row.get("cache_read_tokens"),
                cache_write: row.get("cache_write_tokens"),
            },
            usd_micros: row.get("usd_micros"),
            price_snapshot: PriceSnapshot {
                input_usd_micros_per_million: row.get("input_price"),
                output_usd_micros_per_million: row.get("output_price"),
                cache_read_usd_micros_per_million: row.get("cache_read_price"),
                cache_write_usd_micros_per_million: row.get("cache_write_price"),
            },
        },
        turns: row.get("turns"),
        budget: serde_json::from_value(budget)?,
        stopped: row
            .get::<Option<bool>, _>("stopped")
            .ok_or_else(|| StoreError::Conflict("usage report outcome is still pending".into()))?,
        reason: row.get("reason"),
        usage_event_id: row
            .get::<Option<i64>, _>("usage_event_id")
            .ok_or_else(|| StoreError::Conflict("usage report event is still pending".into()))?,
        claim_failed_event_id: row.get("claim_failed_event_id"),
        accepted_at: row.get("accepted_at"),
    })
}
