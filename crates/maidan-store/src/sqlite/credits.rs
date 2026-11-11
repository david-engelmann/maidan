//! Prepaid credits (`maidan_types::credits`): the ledger, top-up intents,
//! unmatched deposits and the deposit watcher's cursor.
//!
//! SQLite twin of `postgres/credits.rs`. Every write begins with
//! `begin_write`, which holds the database's one write lock from its first
//! statement, so a balance read and the charge that spends it cannot
//! interleave with another writer. The unique indexes of migration 0231 still
//! hold: a replayed log or a second charge from the same point inserts
//! nothing. Timestamps are written as RFC 3339 with milliseconds and `Z`, so
//! they compare as text in time order.

use chrono::SecondsFormat;
use chrono::{DateTime, Utc};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::StoreError;
use maidan_types::{
    usdc_external_ref, ChargeOutcome, CreditChain, CreditCharge, CreditEntry, CreditEntryKind,
    CreditGrant, CreditSource, CreditedDeposit, DepositEffects, DepositLog, DepositOutcome, Event,
    EventKind, NewTopUp, NotificationId, TopUp, TopUpMethod, TopUpRail, TopUpStatus,
    UnmatchedDeposit, UsageMeter, WorkspaceId, OPEN_TOP_UPS_PER_WORKSPACE, TOP_UP_GRACE_SECS,
    TOP_UP_TTL_SECS, USD_MICROS,
};

const LEDGER_COLUMNS: &str =
    "id, workspace_id, kind, source, external_ref, usd_micros, unpaid_micros, meter, month, \
     from_units, through_units, chain, tx_hash, log_index, top_up_id, note, created_at";
const TOP_UP_COLUMNS: &str =
    "id, workspace_id, method, chain, address, checkout_url, usd, amount_micros, status, created_at, \
     expires_at, paid_at";

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

async fn balance_on(
    tx: &mut Transaction<'_, Sqlite>,
    workspace_id: WorkspaceId,
) -> Result<i64, StoreError> {
    let balance: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(usd_micros), 0) FROM maidan_credit_ledger WHERE workspace_id = ?1",
    )
    .bind(workspace_id.0)
    .fetch_one(&mut **tx)
    .await?;
    Ok(balance)
}

/// A workspace's balance: the sum of its ledger.
pub async fn balance(pool: &SqlitePool, workspace_id: WorkspaceId) -> Result<i64, StoreError> {
    let balance: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(usd_micros), 0) FROM maidan_credit_ledger WHERE workspace_id = ?1",
    )
    .bind(workspace_id.0)
    .fetch_one(pool)
    .await?;
    Ok(balance)
}

/// The workspace's most recent ledger rows, newest first.
pub async fn ledger(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    limit: i64,
) -> Result<Vec<CreditEntry>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {LEDGER_COLUMNS} FROM maidan_credit_ledger WHERE workspace_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT ?2"
    ))
    .bind(workspace_id.0)
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_entry).collect()
}

/// Open an intent for `new.usd` dollars plus the first of `new.offsets` no
/// open intent on the chain holds. `Conflict` when the workspace already holds
/// the most open intents it may, or every candidate amount is taken.
pub async fn create_top_up(pool: &SqlitePool, new: &NewTopUp) -> Result<TopUp, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    // Intents past their grace give their amounts back to the instance.
    sqlx::query(
        "UPDATE maidan_credit_top_ups SET status = 'expired'
         WHERE status = 'open' AND expires_at < ?1",
    )
    .bind(ts(new.now - chrono::Duration::seconds(TOP_UP_GRACE_SECS)))
    .execute(&mut *tx)
    .await?;
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maidan_credit_top_ups
         WHERE workspace_id = ?1 AND status = 'open' AND expires_at > ?2",
    )
    .bind(new.workspace_id.0)
    .bind(ts(new.now))
    .fetch_one(&mut *tx)
    .await?;
    if open >= OPEN_TOP_UPS_PER_WORKSPACE {
        return Err(StoreError::Conflict(format!(
            "this workspace has {open} open top-ups, the most it may hold: pay one, or wait for one to expire"
        )));
    }
    let id = uuid::Uuid::now_v7();
    let expires_at = new.now + chrono::Duration::seconds(TOP_UP_TTL_SECS);
    for offset in &new.offsets {
        let amount = new.usd.saturating_mul(USD_MICROS).saturating_add(*offset);
        let inserted = sqlx::query(
            "INSERT INTO maidan_credit_top_ups
                (id, workspace_id, method, chain, address, usd, amount_micros, status, created_at,
                 expires_at)
             VALUES (?1, ?2, 'usdc', ?3, ?4, ?5, ?6, 'open', ?7, ?8)
             ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(new.workspace_id.0)
        .bind(new.chain.as_str())
        .bind(&new.address)
        .bind(new.usd)
        .bind(amount)
        .bind(ts(new.now))
        .bind(ts(expires_at))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 1 {
            tx.commit().await?;
            return Ok(TopUp::from_parts(
                id,
                new.workspace_id,
                TopUpRail::Usdc {
                    chain: new.chain,
                    address: new.address.clone(),
                },
                new.usd,
                amount,
                TopUpStatus::Open,
                new.now,
                expires_at,
                None,
            ));
        }
    }
    Err(StoreError::Conflict(
        "every candidate amount is held by another open top-up; ask again".into(),
    ))
}

/// The workspace's intents still offered at `now`, newest first.
pub async fn open_top_ups(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    now: DateTime<Utc>,
) -> Result<Vec<TopUp>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {TOP_UP_COLUMNS} FROM maidan_credit_top_ups
         WHERE workspace_id = ?1 AND status = 'open' AND expires_at > ?2
         ORDER BY created_at DESC, id DESC"
    ))
    .bind(workspace_id.0)
    .bind(ts(now))
    .fetch_all(pool)
    .await?;
    rows.iter()
        .filter_map(|r| row_to_top_up(r).transpose())
        .collect()
}

/// The workspace an open intent for exactly this transfer belongs to, if any.
/// A peek: [`record_deposit`] matches again under its lock.
pub async fn top_up_workspace(
    pool: &SqlitePool,
    log: &DepositLog,
    now: DateTime<Utc>,
) -> Result<Option<WorkspaceId>, StoreError> {
    let id: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT workspace_id FROM maidan_credit_top_ups
         WHERE status = 'open' AND chain = ?1 AND address = ?2 AND amount_micros = ?3
           AND expires_at > ?4",
    )
    .bind(log.chain.as_str())
    .bind(&log.to_address)
    .bind(log.amount_micros)
    .bind(ts(now - chrono::Duration::seconds(TOP_UP_GRACE_SECS)))
    .fetch_optional(pool)
    .await?;
    Ok(id.map(WorkspaceId))
}

/// Record one transfer: credit the workspace whose open intent asked for
/// exactly this amount and close the intent, or keep it as unmatched. A log
/// seen before changes nothing. On a credit that leaves the balance above
/// zero, this month's crossings on `effects.lift` are removed, so the cutoff
/// lifts; the workspace's admins are told when `effects.admins_of` is the
/// workspace credited.
pub async fn record_deposit(
    pool: &SqlitePool,
    log: &DepositLog,
    effects: &DepositEffects,
    now: DateTime<Utc>,
) -> Result<DepositOutcome, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let seen: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM maidan_credit_ledger
             WHERE source = 'usdc_base' AND external_ref = ?4
         ) OR EXISTS (
             SELECT 1 FROM maidan_credit_unmatched
             WHERE chain = ?1 AND tx_hash = ?2 AND log_index = ?3
         )",
    )
    .bind(log.chain.as_str())
    .bind(&log.tx_hash)
    .bind(log.log_index)
    .bind(usdc_external_ref(&log.tx_hash, log.log_index))
    .fetch_one(&mut *tx)
    .await?;
    if seen {
        return Ok(DepositOutcome::Seen);
    }
    let matched = sqlx::query(
        "SELECT id, workspace_id FROM maidan_credit_top_ups
         WHERE status = 'open' AND chain = ?1 AND address = ?2 AND amount_micros = ?3
           AND expires_at > ?4",
    )
    .bind(log.chain.as_str())
    .bind(&log.to_address)
    .bind(log.amount_micros)
    .bind(ts(now - chrono::Duration::seconds(TOP_UP_GRACE_SECS)))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(matched) = matched else {
        let inserted = sqlx::query(
            "INSERT INTO maidan_credit_unmatched
                (chain, tx_hash, log_index, block_number, from_address, to_address,
                 amount_micros, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT DO NOTHING",
        )
        .bind(log.chain.as_str())
        .bind(&log.tx_hash)
        .bind(log.log_index)
        .bind(log.block_number)
        .bind(&log.from_address)
        .bind(&log.to_address)
        .bind(log.amount_micros)
        .bind(ts(now))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        return Ok(if inserted == 1 {
            DepositOutcome::Unmatched
        } else {
            DepositOutcome::Seen
        });
    };
    let top_up_id: uuid::Uuid = matched.try_get("id")?;
    let workspace_id = WorkspaceId(matched.try_get("workspace_id")?);
    sqlx::query("UPDATE maidan_credit_top_ups SET status = 'paid', paid_at = ?2 WHERE id = ?1")
        .bind(top_up_id)
        .bind(ts(now))
        .execute(&mut *tx)
        .await?;
    let entry_id = uuid::Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO maidan_credit_ledger
            (id, workspace_id, kind, source, external_ref, usd_micros, chain, tx_hash,
             log_index, top_up_id, created_at)
         VALUES (?1, ?2, 'deposit', 'usdc_base', ?9, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT DO NOTHING",
    )
    .bind(entry_id)
    .bind(workspace_id.0)
    .bind(log.amount_micros)
    .bind(log.chain.as_str())
    .bind(&log.tx_hash)
    .bind(log.log_index)
    .bind(top_up_id)
    .bind(ts(now))
    .bind(usdc_external_ref(&log.tx_hash, log.log_index))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(DepositOutcome::Seen);
    }
    let balance_micros = balance_on(&mut tx, workspace_id).await?;
    let mut lifted = Vec::new();
    if balance_micros > 0 && !effects.lift.is_empty() {
        for meter in &effects.lift {
            let removed = sqlx::query(
                "DELETE FROM maidan_usage_over_allowance
                 WHERE workspace_id = ?1 AND month = ?2 AND meter = ?3",
            )
            .bind(workspace_id.0)
            .bind(&effects.month)
            .bind(meter.as_str())
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if removed > 0 {
                lifted.push(*meter);
            }
        }
        lifted.sort();
    }
    let event = if effects.admins_of == Some(workspace_id) {
        let event = Event::CreditsDeposited {
            occurred_at: now,
            workspace_id,
            top_up_id,
            usd_micros: log.amount_micros,
            balance_micros,
            chain: log.chain.as_str().to_string(),
            tx_hash: log.tx_hash.clone(),
        };
        let stored = super::events::append_in_tx(&mut tx, &event).await?;
        for admin in &effects.admins {
            sqlx::query(
                "INSERT INTO maidan_notifications
                    (id, workspace_id, member_id, kind, source_log_id, channel_id, thread_id,
                     message_id, actor_id, created_at, read_at, snoozed_until)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, ?6, NULL, NULL)
                 ON CONFLICT (member_id, source_log_id) DO NOTHING",
            )
            .bind(NotificationId::new().0)
            .bind(workspace_id.0)
            .bind(admin.0)
            .bind(EventKind::CreditsDeposited.as_str())
            .bind(stored.id)
            .bind(ts(now))
            .execute(&mut *tx)
            .await?;
        }
        Some(stored)
    } else {
        None
    };
    tx.commit().await?;
    Ok(DepositOutcome::Credited(Box::new(CreditedDeposit {
        workspace_id,
        top_up_id,
        entry_id,
        balance_micros,
        lifted,
        event,
    })))
}

/// Charge for the units past the allowance on a priced meter that no charge
/// covers yet. Each charge starts where the last one for that workspace,
/// meter and month ended, so a rerun or a second replica charges nothing new.
/// A charge the balance cannot cover is recorded in full, with the uncovered
/// part as unpaid, and leaves the balance below zero.
pub async fn charge(pool: &SqlitePool, charge: &CreditCharge) -> Result<ChargeOutcome, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let from: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(through_units), 0) FROM maidan_credit_ledger
         WHERE kind = 'charge' AND workspace_id = ?1 AND meter = ?2 AND month = ?3",
    )
    .bind(charge.workspace_id.0)
    .bind(charge.meter.as_str())
    .bind(&charge.month)
    .fetch_one(&mut *tx)
    .await?;
    let balance = balance_on(&mut tx, charge.workspace_id).await?;
    let cost = charge
        .price
        .cost_through(charge.through_units)
        .saturating_sub(charge.price.cost_through(from));
    // Nothing new, or units too few to cost a micro yet: they stay uncharged
    // and the next charge, which starts from the same place, covers them.
    if charge.through_units <= from || cost <= 0 {
        return Ok(ChargeOutcome {
            charged_micros: 0,
            unpaid_micros: 0,
            balance_micros: balance,
        });
    }
    let unpaid = cost.saturating_sub(balance.max(0)).max(0);
    let inserted = sqlx::query(
        "INSERT INTO maidan_credit_ledger
            (id, workspace_id, kind, usd_micros, unpaid_micros, meter, month, from_units,
             through_units, created_at)
         VALUES (?1, ?2, 'charge', ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT DO NOTHING",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(charge.workspace_id.0)
    .bind(-cost)
    .bind(unpaid)
    .bind(charge.meter.as_str())
    .bind(&charge.month)
    .bind(from)
    .bind(charge.through_units)
    .bind(ts(charge.at))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(ChargeOutcome {
            charged_micros: 0,
            unpaid_micros: 0,
            balance_micros: balance,
        });
    }
    tx.commit().await?;
    Ok(ChargeOutcome {
        charged_micros: cost,
        unpaid_micros: unpaid,
        balance_micros: balance.saturating_sub(cost),
    })
}

/// Add credit by hand, with the operator's reason.
pub async fn grant(pool: &SqlitePool, grant: &CreditGrant) -> Result<CreditEntry, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_credit_ledger
            (id, workspace_id, kind, source, usd_micros, note, created_at)
         VALUES (?1, ?2, 'grant', 'grant', ?3, ?4, ?5)
         RETURNING {LEDGER_COLUMNS}"
    ))
    .bind(uuid::Uuid::now_v7())
    .bind(grant.workspace_id.0)
    .bind(grant.usd_micros)
    .bind(&grant.note)
    .bind(ts(grant.at))
    .fetch_one(&mut *tx)
    .await?;
    let entry = row_to_entry(&row)?;
    tx.commit().await?;
    Ok(entry)
}

/// Transfers that matched no intent, newest first.
pub async fn unmatched(pool: &SqlitePool, limit: i64) -> Result<Vec<UnmatchedDeposit>, StoreError> {
    let rows = sqlx::query(
        "SELECT chain, tx_hash, log_index, block_number, from_address, to_address,
                amount_micros, seen_at
         FROM maidan_credit_unmatched
         ORDER BY seen_at DESC, block_number DESC, log_index DESC LIMIT ?1",
    )
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let chain: String = row.try_get("chain")?;
        let Some(chain) = CreditChain::parse(&chain) else {
            continue;
        };
        let amount_micros: i64 = row.try_get("amount_micros")?;
        out.push(UnmatchedDeposit {
            chain,
            tx_hash: row.try_get("tx_hash")?,
            log_index: row.try_get("log_index")?,
            block_number: row.try_get("block_number")?,
            from_address: row.try_get("from_address")?,
            to_address: row.try_get("to_address")?,
            amount_micros,
            amount: maidan_types::usd_string(amount_micros),
            seen_at: row.try_get("seen_at")?,
        });
    }
    Ok(out)
}

/// The last block the watcher scanned for this chain and address.
pub async fn cursor(
    pool: &SqlitePool,
    chain: CreditChain,
    address: &str,
) -> Result<Option<i64>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT last_block FROM maidan_credit_cursor WHERE chain = ?1 AND address = ?2",
    )
    .bind(chain.as_str())
    .bind(address)
    .fetch_optional(pool)
    .await?)
}

/// Move the cursor forward to `block`; never back, so a slower replica cannot
/// rewind another's progress.
pub async fn set_cursor(
    pool: &SqlitePool,
    chain: CreditChain,
    address: &str,
    block: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_credit_cursor (chain, address, last_block, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (chain, address) DO UPDATE
         SET last_block = MAX(maidan_credit_cursor.last_block, excluded.last_block),
             updated_at = excluded.updated_at",
    )
    .bind(chain.as_str())
    .bind(address)
    .bind(block)
    .bind(ts(Utc::now()))
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_entry(row: &sqlx::sqlite::SqliteRow) -> Result<CreditEntry, StoreError> {
    let kind: String = row.try_get("kind")?;
    let kind = CreditEntryKind::parse(&kind)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown ledger kind {kind}")))?;
    let meter: Option<String> = row.try_get("meter")?;
    let chain: Option<String> = row.try_get("chain")?;
    let source: Option<String> = row.try_get("source")?;
    let from: Option<i64> = row.try_get("from_units")?;
    let through: Option<i64> = row.try_get("through_units")?;
    Ok(CreditEntry {
        id: row.try_get("id")?,
        workspace_id: WorkspaceId(row.try_get("workspace_id")?),
        kind,
        source: source.as_deref().and_then(CreditSource::parse),
        external_ref: row.try_get("external_ref")?,
        usd_micros: row.try_get("usd_micros")?,
        unpaid_micros: row.try_get("unpaid_micros")?,
        meter: meter.as_deref().and_then(UsageMeter::parse),
        month: row.try_get("month")?,
        units: through.zip(from).map(|(t, f)| t - f),
        through_units: through,
        chain: chain.as_deref().and_then(CreditChain::parse),
        tx_hash: row.try_get("tx_hash")?,
        log_index: row.try_get("log_index")?,
        top_up_id: row.try_get("top_up_id")?,
        note: row.try_get("note")?,
        created_at: row.try_get("created_at")?,
    })
}

/// A row, or `None` for a method, chain or status this build does not know.
fn row_to_top_up(row: &sqlx::sqlite::SqliteRow) -> Result<Option<TopUp>, StoreError> {
    let method: String = row.try_get("method")?;
    let status: String = row.try_get("status")?;
    let Some(status) = TopUpStatus::parse(&status) else {
        return Ok(None);
    };
    let rail = match TopUpMethod::parse(&method) {
        Some(TopUpMethod::Usdc) => {
            let chain: Option<String> = row.try_get("chain")?;
            let address: Option<String> = row.try_get("address")?;
            match (chain.as_deref().and_then(CreditChain::parse), address) {
                (Some(chain), Some(address)) => TopUpRail::Usdc { chain, address },
                _ => return Ok(None),
            }
        }
        Some(TopUpMethod::Card) => TopUpRail::Card {
            checkout_url: row.try_get("checkout_url")?,
        },
        None => return Ok(None),
    };
    Ok(Some(TopUp::from_parts(
        row.try_get("id")?,
        WorkspaceId(row.try_get("workspace_id")?),
        rail,
        row.try_get("usd")?,
        row.try_get("amount_micros")?,
        status,
        row.try_get("created_at")?,
        row.try_get("expires_at")?,
        row.try_get("paid_at")?,
    )))
}
