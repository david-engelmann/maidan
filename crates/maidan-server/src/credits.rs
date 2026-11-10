//! Prepaid credits paid in USDC on Base (`docs/Credits.md`): the instance's
//! terms from the environment, and the deposit watcher.
//!
//! The watcher only reads the chain. It asks a JSON-RPC endpoint for the head
//! block and for USDC `Transfer` logs to the receiving address, in ranges of
//! at most [`MAX_RANGE`] blocks that are at least
//! `MAIDAN_CREDITS_CONFIRMATIONS` blocks deep, and records each log in the
//! store, which credits the workspace whose open intent asked for exactly that
//! amount or keeps the transfer as unmatched. It holds no key and sends no
//! transaction. Every replica may run it: the store records a log once, and
//! the cursor only moves forward.

use std::sync::Arc;
use std::time::Duration;

use maidan_types::{
    normalize_address, CreditChain, CreditTerms, CreditsSettings, DepositEffects, DepositLog,
    DepositOutcome, OveragePrices, UsageMonth, TRANSFER_TOPIC,
};
use serde_json::{json, Value};

use crate::state::AppState;

/// Confirmations a transfer waits for by default.
pub const DEFAULT_CONFIRMATIONS: u64 = 5;
/// How often the watcher polls by default.
pub const DEFAULT_POLL_SECS: u64 = 15;
/// The most blocks one `eth_getLogs` call asks for: public endpoints refuse
/// wider ranges.
pub const MAX_RANGE: u64 = 1_000;
/// Ranges scanned per tick, so a watcher far behind catches up over several
/// ticks rather than holding one for minutes.
pub const RANGES_PER_TICK: u64 = 20;

/// How the watcher reaches the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatcherConfig {
    pub settings: CreditsSettings,
    pub rpc_url: String,
    pub confirmations: u64,
    pub interval: Duration,
}

/// The instance's credit terms and, when it takes payment, the watcher's
/// configuration, from the environment. A malformed value refuses boot,
/// naming the variable.
pub fn from_env() -> Result<(CreditTerms, Option<WatcherConfig>), String> {
    from_lookup(|name| std::env::var(name).ok())
}

/// [`from_env`] over any lookup, for tests.
pub fn from_lookup(
    get: impl Fn(&str) -> Option<String>,
) -> Result<(CreditTerms, Option<WatcherConfig>), String> {
    let set = |name: &str| get(name).filter(|v| !v.trim().is_empty());
    let prices = match set("MAIDAN_OVERAGE_PRICES") {
        Some(raw) => {
            OveragePrices::parse(&raw).map_err(|err| format!("MAIDAN_OVERAGE_PRICES: {err}"))?
        }
        None => OveragePrices::default(),
    };
    let Some(address) = set("MAIDAN_CREDITS_ADDRESS") else {
        for name in [
            "MAIDAN_CREDITS_CHAIN",
            "MAIDAN_CREDITS_RPC_URL",
            "MAIDAN_CREDITS_CONFIRMATIONS",
            "MAIDAN_CREDITS_POLL_SECS",
        ] {
            if set(name).is_some() {
                tracing::warn!(
                    variable = name,
                    "set without MAIDAN_CREDITS_ADDRESS, so credits are off and it is ignored"
                );
            }
        }
        return Ok((
            CreditTerms {
                settings: None,
                prices,
            },
            None,
        ));
    };
    let settings = CreditsSettings::parse(set("MAIDAN_CREDITS_CHAIN").as_deref(), &address)
        .map_err(|err| format!("MAIDAN_CREDITS_ADDRESS / MAIDAN_CREDITS_CHAIN: {err}"))?;
    let rpc_url = match set("MAIDAN_CREDITS_RPC_URL") {
        Some(url) => {
            let url = url.trim().to_string();
            let parsed = reqwest::Url::parse(&url)
                .map_err(|err| format!("MAIDAN_CREDITS_RPC_URL: `{url}` is not a URL: {err}"))?;
            if !matches!(parsed.scheme(), "https" | "http") {
                return Err(format!(
                    "MAIDAN_CREDITS_RPC_URL: `{url}` is not an http(s) URL"
                ));
            }
            url
        }
        None => settings.chain.default_rpc_url().to_string(),
    };
    let whole = |name: &str, default: u64, min: u64| -> Result<u64, String> {
        match set(name) {
            None => Ok(default),
            Some(raw) => raw
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|n| *n >= min)
                .ok_or_else(|| {
                    format!(
                        "{name}: `{}` is not a whole number of at least {min}",
                        raw.trim()
                    )
                }),
        }
    };
    let confirmations = whole("MAIDAN_CREDITS_CONFIRMATIONS", DEFAULT_CONFIRMATIONS, 1)?;
    let interval = Duration::from_secs(whole("MAIDAN_CREDITS_POLL_SECS", DEFAULT_POLL_SECS, 1)?);
    Ok((
        CreditTerms {
            settings: Some(settings.clone()),
            prices,
        },
        Some(WatcherConfig {
            settings,
            rpc_url,
            confirmations,
            interval,
        }),
    ))
}

/// Why a read of the chain failed. The watcher logs it and tries again next
/// tick, from the same block.
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("rpc transport: {0}")]
    Transport(String),
    #[error("rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("malformed rpc answer: {0}")]
    Malformed(String),
}

/// One ERC-20 `Transfer` log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferLog {
    /// Lowercase.
    pub tx_hash: String,
    pub log_index: u64,
    pub block_number: u64,
    /// Lowercase.
    pub from: String,
    /// Lowercase.
    pub to: String,
    /// In token base units; saturates at `i64::MAX`, far above any USDC
    /// supply.
    pub amount: i64,
    /// Dropped by a reorganisation.
    pub removed: bool,
}

/// What the watcher reads the chain through. The JSON-RPC client is the
/// real one; tests use a fake, so CI never reaches a chain.
#[async_trait::async_trait]
pub trait ChainReader: Send + Sync {
    /// The head block number (`eth_blockNumber`).
    async fn block_number(&self) -> Result<u64, ChainError>;
    /// `Transfer` logs of `token` to `to` in blocks `from..=until`
    /// (`eth_getLogs`).
    async fn transfer_logs(
        &self,
        token: &str,
        to: &str,
        from: u64,
        until: u64,
    ) -> Result<Vec<TransferLog>, ChainError>;
}

/// A JSON-RPC endpoint over HTTP.
pub struct JsonRpcReader {
    client: reqwest::Client,
    url: String,
}

impl JsonRpcReader {
    pub fn new(url: &str) -> Result<Self, String> {
        let client = crate::egress_http::bounded()
            .user_agent(concat!("maidan-credits/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| err.to_string())?;
        Ok(Self {
            client,
            url: url.to_string(),
        })
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, ChainError> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let response = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|err| ChainError::Transport(err.without_url().to_string()))?;
        let status = response.status();
        let answer: Value = response
            .json()
            .await
            .map_err(|err| ChainError::Transport(format!("{status}: {}", err.without_url())))?;
        if let Some(error) = answer.get("error") {
            return Err(ChainError::Rpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect(),
            });
        }
        answer
            .get("result")
            .cloned()
            .ok_or_else(|| ChainError::Malformed(format!("{method}: no result ({status})")))
    }
}

#[async_trait::async_trait]
impl ChainReader for JsonRpcReader {
    async fn block_number(&self) -> Result<u64, ChainError> {
        let head = self.call("eth_blockNumber", json!([])).await?;
        quantity(&head).ok_or_else(|| ChainError::Malformed(format!("block number {head}")))
    }

    async fn transfer_logs(
        &self,
        token: &str,
        to: &str,
        from: u64,
        until: u64,
    ) -> Result<Vec<TransferLog>, ChainError> {
        let filter = json!([{
            "fromBlock": format!("0x{from:x}"),
            "toBlock": format!("0x{until:x}"),
            "address": token,
            "topics": [TRANSFER_TOPIC, Value::Null, topic_address(to)],
        }]);
        let logs = self.call("eth_getLogs", filter).await?;
        logs.as_array()
            .ok_or_else(|| ChainError::Malformed("eth_getLogs did not answer a list".into()))?
            .iter()
            .map(parse_log)
            .collect()
    }
}

/// An address as a 32-byte log topic.
fn topic_address(address: &str) -> String {
    format!("0x{:0>64}", address.trim_start_matches("0x"))
}

/// A hex quantity (`0x1a`).
fn quantity(value: &Value) -> Option<u64> {
    u64::from_str_radix(value.as_str()?.strip_prefix("0x")?, 16).ok()
}

/// The address in the low 20 bytes of a 32-byte topic.
fn topic_to_address(topic: &str) -> Option<String> {
    let hex = topic.strip_prefix("0x")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    normalize_address(&format!("0x{}", &hex[24..]))
}

/// A 32-byte big-endian amount, saturating at `i64::MAX`.
fn word_amount(data: &str) -> Option<i64> {
    let hex = data.strip_prefix("0x")?;
    if hex.is_empty() || hex.len() > 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digits = hex.trim_start_matches('0');
    if digits.len() > 16 {
        return Some(i64::MAX);
    }
    let value = u64::from_str_radix(if digits.is_empty() { "0" } else { digits }, 16).ok()?;
    Some(i64::try_from(value).unwrap_or(i64::MAX))
}

/// One `eth_getLogs` entry as a transfer. A log that is not an ERC-20
/// `Transfer` (wrong topic count or topic) is malformed: the filter asked for
/// those only.
pub fn parse_log(log: &Value) -> Result<TransferLog, ChainError> {
    let malformed = |what: &str| ChainError::Malformed(format!("log {what}: {log}"));
    let topics = log
        .get("topics")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("topics"))?;
    let topic = |i: usize| topics.get(i).and_then(Value::as_str);
    if topics.len() != 3 || !topic(0).is_some_and(|t| t.eq_ignore_ascii_case(TRANSFER_TOPIC)) {
        return Err(malformed("is not a Transfer"));
    }
    let from = topic(1)
        .and_then(topic_to_address)
        .ok_or_else(|| malformed("from"))?;
    let to = topic(2)
        .and_then(topic_to_address)
        .ok_or_else(|| malformed("to"))?;
    let amount = log
        .get("data")
        .and_then(Value::as_str)
        .and_then(word_amount)
        .ok_or_else(|| malformed("amount"))?;
    let tx_hash = log
        .get("transactionHash")
        .and_then(Value::as_str)
        .filter(|h| h.len() == 66 && h.starts_with("0x"))
        .ok_or_else(|| malformed("transaction hash"))?
        .to_ascii_lowercase();
    let log_index = log
        .get("logIndex")
        .and_then(quantity)
        .ok_or_else(|| malformed("index"))?;
    let block_number = log
        .get("blockNumber")
        .and_then(quantity)
        .ok_or_else(|| malformed("block"))?;
    Ok(TransferLog {
        tx_hash,
        log_index,
        block_number,
        from,
        to,
        amount,
        removed: log.get("removed").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// What one tick did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickReport {
    /// The last block scanned, when the tick scanned any.
    pub scanned_to: Option<u64>,
    pub credited: usize,
    pub unmatched: usize,
}

/// Why a tick stopped early. It starts again from the same block next time,
/// and the store records each log once, so nothing is lost or doubled.
#[derive(Debug, thiserror::Error)]
pub enum TickError {
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error(transparent)]
    Store(#[from] maidan_store::StoreError),
}

/// Scan confirmed blocks past the cursor, at most [`RANGES_PER_TICK`] ranges
/// of [`MAX_RANGE`] blocks, recording every transfer to the receiving
/// address. A first run, with no cursor, starts [`MAX_RANGE`] blocks back.
pub async fn tick(
    state: &AppState,
    reader: &dyn ChainReader,
    config: &WatcherConfig,
) -> Result<TickReport, TickError> {
    let settings = &config.settings;
    let mut report = TickReport::default();
    let head = reader.block_number().await?;
    let Some(safe) = head.checked_sub(config.confirmations) else {
        return Ok(report);
    };
    let mut start = match state
        .store
        .credit_cursor(settings.chain, &settings.address)
        .await?
    {
        Some(last) => u64::try_from(last).unwrap_or(0).saturating_add(1),
        None => safe.saturating_sub(MAX_RANGE - 1),
    };
    let mut lifted = false;
    for _ in 0..RANGES_PER_TICK {
        if start > safe {
            break;
        }
        let until = safe.min(start.saturating_add(MAX_RANGE - 1));
        let mut logs = reader
            .transfer_logs(settings.chain.usdc(), &settings.address, start, until)
            .await?;
        logs.sort_by_key(|log| (log.block_number, log.log_index));
        for log in logs {
            if log.removed || log.to != settings.address {
                continue;
            }
            match record(state, settings.chain, &log).await? {
                DepositOutcome::Credited(credited) => {
                    report.credited += 1;
                    lifted |= !credited.lifted.is_empty();
                    tracing::info!(
                        workspace_id = %credited.workspace_id.0,
                        top_up_id = %credited.top_up_id,
                        usd_micros = log.amount,
                        tx_hash = %log.tx_hash,
                        "credits: deposit credited"
                    );
                    if let Some(event) = credited.event {
                        crate::routes::publish_stored(state, event).await;
                    }
                }
                DepositOutcome::Unmatched => {
                    report.unmatched += 1;
                    tracing::warn!(
                        usd_micros = log.amount,
                        tx_hash = %log.tx_hash,
                        log_index = log.log_index,
                        "credits: a transfer matched no open top-up; kept for the operator"
                    );
                }
                DepositOutcome::Seen => {}
            }
        }
        state
            .store
            .set_credit_cursor(
                settings.chain,
                &settings.address,
                i64::try_from(until).unwrap_or(i64::MAX),
            )
            .await?;
        report.scanned_to = Some(until);
        start = until.saturating_add(1);
    }
    if lifted {
        // This replica refuses no longer at once; the others on their next
        // usage-meter tick.
        crate::usage_meters::refresh_cut_off(state).await;
    }
    Ok(report)
}

/// Record one transfer, telling the admins of the workspace it is expected to
/// credit.
async fn record(
    state: &AppState,
    chain: CreditChain,
    log: &TransferLog,
) -> Result<DepositOutcome, TickError> {
    let now = chrono::Utc::now();
    let deposit = DepositLog {
        chain,
        tx_hash: log.tx_hash.clone(),
        log_index: i64::try_from(log.log_index).unwrap_or(i64::MAX),
        block_number: i64::try_from(log.block_number).unwrap_or(i64::MAX),
        from_address: log.from.clone(),
        to_address: log.to.clone(),
        amount_micros: log.amount,
    };
    let expected = state
        .store
        .find_credit_top_up_workspace(&deposit, now)
        .await?;
    let admins = match expected {
        Some(workspace_id) => crate::usage_meters::workspace_admins(state, workspace_id).await?,
        None => Vec::new(),
    };
    let effects = DepositEffects {
        month: UsageMonth::of(now.date_naive()).label(),
        lift: state.credit_terms.priced_meters(),
        admins_of: expected,
        admins,
    };
    Ok(state
        .store
        .record_credit_deposit(&deposit, &effects, now)
        .await?)
}

/// The watcher loop, spawned by `main.rs` when `MAIDAN_CREDITS_ADDRESS` is
/// set.
pub async fn run(state: AppState, reader: Arc<dyn ChainReader>, config: WatcherConfig) {
    tracing::info!(
        chain = config.settings.chain.as_str(),
        address = %config.settings.address,
        confirmations = config.confirmations,
        interval_secs = config.interval.as_secs(),
        "credits deposit watcher started"
    );
    let _watched = crate::ops_alert::start_loop("credits_watcher", config.interval);
    loop {
        match crate::ops_alert::guarded_tick(
            "credits_watcher",
            tick(&state, reader.as_ref(), &config),
        )
        .await
        {
            Some(Ok(report)) => tracing::debug!(?report, "credits watcher tick"),
            Some(Err(err)) => {
                tracing::warn!(error = %err, "credits watcher: tick stopped; it resumes from the same block")
            }
            None => {}
        }
        tokio::time::sleep(config.interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn credits_are_off_without_an_address_and_malformed_values_refuse_boot() {
        let (terms, watcher) = from_lookup(env(&[])).unwrap();
        assert_eq!((terms, watcher), (CreditTerms::default(), None));
        let (terms, watcher) = from_lookup(env(&[(
            "MAIDAN_OVERAGE_PRICES",
            "mcp_calls=1000000/100000",
        )]))
        .unwrap();
        assert!(terms.settings.is_none() && watcher.is_none());
        assert_eq!(terms.price(maidan_types::UsageMeter::McpCalls), None);

        let address = "0x00000000000000000000000000000000000000AA";
        let (terms, watcher) = from_lookup(env(&[
            ("MAIDAN_CREDITS_ADDRESS", address),
            ("MAIDAN_CREDITS_CHAIN", "base-sepolia"),
            ("MAIDAN_OVERAGE_PRICES", "mcp_calls=1000000/100000"),
        ]))
        .unwrap();
        let watcher = watcher.unwrap();
        assert_eq!(watcher.rpc_url, "https://sepolia.base.org");
        assert_eq!(watcher.confirmations, DEFAULT_CONFIRMATIONS);
        assert_eq!(
            watcher.settings.address,
            "0x00000000000000000000000000000000000000aa"
        );
        assert!(terms.price(maidan_types::UsageMeter::McpCalls).is_some());

        for bad in [
            vec![("MAIDAN_CREDITS_ADDRESS", "0x1234")],
            vec![
                ("MAIDAN_CREDITS_ADDRESS", address),
                ("MAIDAN_CREDITS_CHAIN", "eth"),
            ],
            vec![
                ("MAIDAN_CREDITS_ADDRESS", address),
                ("MAIDAN_CREDITS_RPC_URL", "ftp://x"),
            ],
            vec![
                ("MAIDAN_CREDITS_ADDRESS", address),
                ("MAIDAN_CREDITS_CONFIRMATIONS", "0"),
            ],
            vec![
                ("MAIDAN_CREDITS_ADDRESS", address),
                ("MAIDAN_CREDITS_POLL_SECS", "soon"),
            ],
            vec![("MAIDAN_OVERAGE_PRICES", "stored_bytes=1/1")],
            vec![("MAIDAN_OVERAGE_PRICES", "mcp_calls=1")],
        ] {
            assert!(from_lookup(env(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_transfer_log_parses_from_its_topics_and_data() {
        // A real Base mainnet USDC Transfer log (block 0x3531652).
        let log = json!({
            "address": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                "0x000000000000000000000000958d706a46431ce302d2f9b43a1624d2147bcb5a",
                "0x000000000000000000000000b4bb2032a73a53c0aa7dc9ee2d9658a978fa7bc2"
            ],
            "data": "0x0000000000000000000000000000000000000000000000000000000001c5a4b1",
            "blockNumber": "0x3531652",
            "transactionHash": "0x54e2797a303047b8271f8c7cd19947e4e96eda740a01591dc2839d7b8820d997",
            "logIndex": "0x7",
            "removed": false
        });
        let parsed = parse_log(&log).unwrap();
        assert_eq!(parsed.from, "0x958d706a46431ce302d2f9b43a1624d2147bcb5a");
        assert_eq!(parsed.to, "0xb4bb2032a73a53c0aa7dc9ee2d9658a978fa7bc2");
        assert_eq!(parsed.amount, 0x01c5a4b1);
        assert_eq!((parsed.block_number, parsed.log_index), (0x3531652, 7));
        assert!(!parsed.removed);

        let mut huge = log.clone();
        huge["data"] = json!(format!("0x{}", "f".repeat(64)));
        assert_eq!(parse_log(&huge).unwrap().amount, i64::MAX);
        let mut approval = log.clone();
        approval["topics"][0] =
            json!("0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925");
        assert!(parse_log(&approval).is_err());
        let mut short = log;
        short["topics"][2] = json!("0x1234");
        assert!(parse_log(&short).is_err());
        assert_eq!(
            topic_address("0x00000000000000000000000000000000000000aa"),
            format!("0x{}aa", "0".repeat(62))
        );
    }
}
