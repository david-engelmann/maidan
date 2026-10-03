use maidan_types::{
    ClaimLeaseId, MemberId, NewUsageLedgerEntry, PayerStamp, PriceSnapshot, ThreadBudget, ThreadId,
    TokenUsage, UsageEvidence, UsageLedgerEntry, UsageRollup, UsageRollupQuery, UsageSums,
    WorkspaceId,
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

pub async fn rollup(pool: &PgPool, query: UsageRollupQuery) -> Result<UsageRollup, StoreError> {
    query.scope_name().map_err(StoreError::InvalidInput)?;
    let (filter, thread, member) = scope_filter(pool, query).await?;
    let row = sqlx::query(&format!(
        "SELECT
            COUNT(*) AS reports,
            COALESCE(SUM(input_tokens), 0)::bigint AS input_tokens,
            COALESCE(SUM(output_tokens), 0)::bigint AS output_tokens,
            COALESCE(SUM(cache_read_tokens), 0)::bigint AS cache_read_tokens,
            COALESCE(SUM(cache_write_5m_tokens), 0)::bigint AS cache_write_5m_tokens,
            COALESCE(SUM(cache_write_1h_tokens), 0)::bigint AS cache_write_1h_tokens,
            COALESCE(SUM(usd_micros), 0)::bigint AS usd_micros,
            COALESCE(SUM(
                ((input_tokens + cache_read_tokens + cache_write_5m_tokens + cache_write_1h_tokens) * input_price
                 + output_tokens * output_price + 999999) / 1000000
            ), 0)::bigint AS uncached_usd_micros
         FROM maidan_usage_ledger
         WHERE budget IS NOT NULL AND {filter}"
    ))
    .bind(query.workspace_id.0)
    .bind(thread)
    .bind(member)
    .fetch_one(pool)
    .await?;
    let sums = UsageSums {
        reports: row.get("reports"),
        input_tokens: row.get("input_tokens"),
        output_tokens: row.get("output_tokens"),
        cache_read_tokens: row.get("cache_read_tokens"),
        cache_write_5m_tokens: row.get("cache_write_5m_tokens"),
        cache_write_1h_tokens: row.get("cache_write_1h_tokens"),
        usd_micros: row.get("usd_micros"),
        uncached_usd_micros: row.get("uncached_usd_micros"),
    };
    let (completed_tasks, completed_usd) = completed(pool, query).await?;
    UsageRollup::from_sums(query, sums, completed_tasks, completed_usd)
        .map_err(StoreError::InvalidInput)
}

async fn scope_filter(
    pool: &PgPool,
    query: UsageRollupQuery,
) -> Result<(&'static str, Option<uuid::Uuid>, Option<uuid::Uuid>), StoreError> {
    // A null bind means that column is not a filter. SQLite spells null-safe
    // inequality `IS NOT`; Postgres spells it `IS DISTINCT FROM`.
    match (query.thread_id, query.member_id) {
        (Some(thread_id), None) => {
            let row = sqlx::query(
                "SELECT c.workspace_id
                 FROM maidan_threads t
                 JOIN maidan_channels c ON c.id = t.channel_id
                 WHERE t.id = $1",
            )
            .bind(thread_id.0)
            .fetch_optional(pool)
            .await?
            .ok_or(StoreError::NotFound)?;
            let workspace = WorkspaceId(row.get("workspace_id"));
            if workspace != query.workspace_id {
                return Err(StoreError::NotFound);
            }
            Ok((
                "workspace_id = $1 AND thread_id = $2 AND reporter_id IS DISTINCT FROM $3",
                Some(thread_id.0),
                None,
            ))
        }
        (None, Some(member_id)) => {
            let row = sqlx::query("SELECT workspace_id FROM maidan_members WHERE id = $1")
                .bind(member_id.0)
                .fetch_optional(pool)
                .await?
                .ok_or(StoreError::NotFound)?;
            if WorkspaceId(row.get("workspace_id")) != query.workspace_id {
                return Err(StoreError::NotFound);
            }
            Ok((
                "workspace_id = $1 AND thread_id IS DISTINCT FROM $2 AND reporter_id = $3",
                None,
                Some(member_id.0),
            ))
        }
        (None, None) => Ok((
            "workspace_id = $1 AND thread_id IS DISTINCT FROM $2 AND reporter_id IS DISTINCT FROM $3",
            None,
            None,
        )),
        (Some(_), Some(_)) => Err(StoreError::InvalidInput(
            "name thread_id or member_id, not both".into(),
        )),
    }
}

async fn completed(pool: &PgPool, query: UsageRollupQuery) -> Result<(i64, i64), StoreError> {
    match (query.thread_id, query.member_id) {
        (Some(thread_id), None) => {
            let row = sqlx::query(
                "SELECT state, tombstoned_at IS NULL AS live
                 FROM maidan_threads WHERE id = $1",
            )
            .bind(thread_id.0)
            .fetch_optional(pool)
            .await?
            .ok_or(StoreError::NotFound)?;
            let state: String = row.get("state");
            let live: bool = row.get("live");
            let done = live && matches!(state.as_str(), "closed" | "archived");
            if !done {
                return Ok((0, 0));
            }
            let usd: i64 = sqlx::query(
                "SELECT COALESCE(SUM(usd_micros), 0)::bigint AS usd
                 FROM maidan_usage_ledger
                 WHERE thread_id = $1 AND budget IS NOT NULL",
            )
            .bind(thread_id.0)
            .fetch_one(pool)
            .await?
            .get("usd");
            Ok((1, usd))
        }
        (None, Some(member_id)) => {
            let row = sqlx::query(
                "SELECT COUNT(DISTINCT t.id) AS tasks,
                        COALESCE(SUM(l.usd_micros), 0)::bigint AS usd
                 FROM maidan_usage_ledger l
                 JOIN maidan_threads t ON t.id = l.thread_id
                 WHERE l.workspace_id = $1 AND l.reporter_id = $2 AND l.budget IS NOT NULL
                   AND t.tombstoned_at IS NULL AND t.state IN ('closed', 'archived')",
            )
            .bind(query.workspace_id.0)
            .bind(member_id.0)
            .fetch_one(pool)
            .await?;
            Ok((row.get("tasks"), row.get("usd")))
        }
        (None, None) => {
            let tasks: i64 = sqlx::query(
                "SELECT COUNT(*) AS tasks
                 FROM maidan_threads t
                 JOIN maidan_channels c ON c.id = t.channel_id
                 WHERE c.workspace_id = $1 AND t.tombstoned_at IS NULL
                   AND t.state IN ('closed', 'archived')",
            )
            .bind(query.workspace_id.0)
            .fetch_one(pool)
            .await?
            .get("tasks");
            let usd: i64 = sqlx::query(
                "SELECT COALESCE(SUM(l.usd_micros), 0)::bigint AS usd
                 FROM maidan_usage_ledger l
                 JOIN maidan_threads t ON t.id = l.thread_id
                 WHERE l.workspace_id = $1 AND l.budget IS NOT NULL
                   AND t.tombstoned_at IS NULL AND t.state IN ('closed', 'archived')",
            )
            .bind(query.workspace_id.0)
            .fetch_one(pool)
            .await?
            .get("usd");
            Ok((tasks, usd))
        }
        (Some(_), Some(_)) => Err(StoreError::InvalidInput(
            "name thread_id or member_id, not both".into(),
        )),
    }
}

pub(super) async fn reserve_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    new: &NewUsageLedgerEntry,
    workspace_id: WorkspaceId,
) -> Result<bool, StoreError> {
    let packs = packs_json(&new.evidence)?;
    let result = sqlx::query(
        "INSERT INTO maidan_usage_ledger (
            usage_report_id, workspace_id, thread_id, reporter_id,
            claim_lease_id, model, input_tokens, output_tokens,
            cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens,
            input_price, output_price, cache_read_price,
            cache_write_5m_price, cache_write_1h_price, usd_micros, turns,
            provider, service_tier, batch, harness, harness_version,
            cache_key, cache_miss_reason, pack_sha256
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26)
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
    .bind(new.tokens.cache_write_5m)
    .bind(new.tokens.cache_write_1h)
    .bind(new.price_snapshot.input_usd_micros_per_million)
    .bind(new.price_snapshot.output_usd_micros_per_million)
    .bind(new.price_snapshot.cache_read_usd_micros_per_million)
    .bind(new.price_snapshot.cache_write_5m_usd_micros_per_million)
    .bind(new.price_snapshot.cache_write_1h_usd_micros_per_million)
    .bind(new.usd_micros)
    .bind(new.turns)
    .bind(new.evidence.provider.as_deref())
    .bind(new.evidence.service_tier.as_deref())
    .bind(new.evidence.batch)
    .bind(new.evidence.harness.as_deref())
    .bind(new.evidence.harness_version.as_deref())
    .bind(new.evidence.cache_key.as_deref())
    .bind(new.evidence.cache_miss_reason.as_deref())
    .bind(packs)
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
            budget = $1, stopped = $2, reason = $3, usage_event_id = $4,
            claim_failed_event_id = $5
         WHERE usage_report_id = $6 RETURNING *",
    )
    .bind(budget)
    .bind(stopped)
    .bind(reason)
    .bind(usage_event_id)
    .bind(claim_failed_event_id)
    .bind(usage_report_id)
    .fetch_one(&mut **tx)
    .await?;
    row_to_entry(&row)
}

fn packs_json(evidence: &UsageEvidence) -> Result<Option<String>, StoreError> {
    if evidence.pack_sha256.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(&evidence.pack_sha256)?))
}

fn row_to_entry(row: &sqlx::postgres::PgRow) -> Result<UsageLedgerEntry, StoreError> {
    let budget = row
        .get::<Option<serde_json::Value>, _>("budget")
        .ok_or_else(|| StoreError::Conflict("usage report is still pending".into()))?;
    let packs: Vec<String> = match row.get::<Option<String>, _>("pack_sha256") {
        None => Vec::new(),
        Some(text) => serde_json::from_str(&text)?,
    };
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
                cache_write_5m: row.get("cache_write_5m_tokens"),
                cache_write_1h: row.get("cache_write_1h_tokens"),
            },
            usd_micros: row.get("usd_micros"),
            price_snapshot: PriceSnapshot {
                input_usd_micros_per_million: row.get("input_price"),
                output_usd_micros_per_million: row.get("output_price"),
                cache_read_usd_micros_per_million: row.get("cache_read_price"),
                cache_write_5m_usd_micros_per_million: row.get("cache_write_5m_price"),
                cache_write_1h_usd_micros_per_million: row.get("cache_write_1h_price"),
            },
        },
        turns: row.get("turns"),
        evidence: UsageEvidence {
            provider: row.get("provider"),
            service_tier: row.get("service_tier"),
            batch: row.get("batch"),
            harness: row.get("harness"),
            harness_version: row.get("harness_version"),
            cache_key: row.get("cache_key"),
            cache_miss_reason: row.get("cache_miss_reason"),
            pack_sha256: packs,
        },
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
