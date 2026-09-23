//! Accountable, retry-safe model-usage records.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ClaimLeaseId, MemberId, ThreadBudget, ThreadId, WorkspaceId};

/// Token quantities billed at independently snapshotted rates.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl TokenUsage {
    pub fn total(self) -> Result<i64, String> {
        let values = [self.input, self.output, self.cache_read, self.cache_write];
        if values.iter().any(|value| *value < 0) {
            return Err("token counts must be non-negative".into());
        }
        values
            .into_iter()
            .try_fold(0_i64, i64::checked_add)
            .ok_or_else(|| "token total overflow".into())
    }
}

/// Immutable prices used for one report, in micro-USD per million tokens.
///
/// Maidan preserves this reporter-supplied evidence; it is not a live vendor
/// rate card. The four explicit tiers prevent cached tokens from being silently
/// charged at the input rate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PriceSnapshot {
    pub input_usd_micros_per_million: i64,
    pub output_usd_micros_per_million: i64,
    pub cache_read_usd_micros_per_million: i64,
    pub cache_write_usd_micros_per_million: i64,
}

impl PriceSnapshot {
    /// Compute the charge in micro-USD, rounding a fractional micro-dollar up.
    pub fn charge_usd_micros(self, tokens: TokenUsage) -> Result<i64, String> {
        let rates = [
            self.input_usd_micros_per_million,
            self.output_usd_micros_per_million,
            self.cache_read_usd_micros_per_million,
            self.cache_write_usd_micros_per_million,
        ];
        if rates.iter().any(|rate| *rate < 0) {
            return Err("price snapshot rates must be non-negative".into());
        }
        tokens.total()?;
        let quantities = [
            tokens.input,
            tokens.output,
            tokens.cache_read,
            tokens.cache_write,
        ];
        let numerator = quantities
            .into_iter()
            .zip(rates)
            .try_fold(0_i128, |total, (quantity, rate)| {
                let line = i128::from(quantity).checked_mul(i128::from(rate))?;
                total.checked_add(line)
            })
            .ok_or_else(|| "price calculation overflow".to_string())?;
        let rounded = numerator
            .checked_add(999_999)
            .ok_or_else(|| "price calculation overflow".to_string())?
            / 1_000_000;
        i64::try_from(rounded).map_err(|_| "USD charge overflow".into())
    }
}

/// Immutable attribution carried by the usage ledger and `UsageReported`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PayerStamp {
    pub payer: WorkspaceId,
    pub reporter: MemberId,
    pub claim_lease_id: ClaimLeaseId,
    pub model: String,
    pub tokens: TokenUsage,
    pub usd_micros: i64,
    pub price_snapshot: PriceSnapshot,
}

/// Caller-supplied economic evidence for one retry-safe usage heartbeat.
///
/// The thread comes from the REST path or MCP tool envelope. The reporter and
/// payer are deliberately absent: Maidan derives them from authentication and
/// the resolved thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct AccountedUsageRequest {
    pub usage_report_id: uuid::Uuid,
    pub claim_lease_id: ClaimLeaseId,
    pub model: String,
    pub tokens: TokenUsage,
    pub usd_micros: i64,
    pub price_snapshot: PriceSnapshot,
    #[serde(default)]
    pub turns: i64,
}

impl AccountedUsageRequest {
    pub fn into_new(self, thread_id: ThreadId, reporter: MemberId) -> NewUsageLedgerEntry {
        NewUsageLedgerEntry {
            usage_report_id: self.usage_report_id,
            thread_id,
            reporter,
            claim_lease_id: self.claim_lease_id,
            model: self.model,
            tokens: self.tokens,
            usd_micros: self.usd_micros,
            price_snapshot: self.price_snapshot,
            turns: self.turns,
        }
    }
}

/// Input to the store's idempotent accounted-usage operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct NewUsageLedgerEntry {
    pub usage_report_id: uuid::Uuid,
    pub thread_id: ThreadId,
    pub reporter: MemberId,
    pub claim_lease_id: ClaimLeaseId,
    pub model: String,
    pub tokens: TokenUsage,
    pub usd_micros: i64,
    pub price_snapshot: PriceSnapshot,
    #[serde(default)]
    pub turns: i64,
}

impl NewUsageLedgerEntry {
    pub fn validate(&self) -> Result<(), String> {
        let model = self.model.trim();
        if model.is_empty() || model.len() > 255 {
            return Err("model must contain 1..=255 bytes after trimming".into());
        }
        if self.turns < 0 {
            return Err("turns must be non-negative".into());
        }
        if self.usd_micros < 0 {
            return Err("usd_micros must be non-negative".into());
        }
        let computed = self.price_snapshot.charge_usd_micros(self.tokens)?;
        if computed != self.usd_micros {
            return Err(format!(
                "usd_micros {actual} does not match price snapshot calculation {computed}",
                actual = self.usd_micros
            ));
        }
        Ok(())
    }
}

/// Durable accepted usage plus the outcome returned for exact retries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageLedgerEntry {
    pub usage_report_id: uuid::Uuid,
    pub thread_id: ThreadId,
    pub stamp: PayerStamp,
    pub turns: i64,
    pub budget: ThreadBudget,
    pub stopped: bool,
    pub reason: Option<String>,
    pub usage_event_id: i64,
    pub claim_failed_event_id: Option<i64>,
    pub accepted_at: DateTime<Utc>,
}

impl UsageLedgerEntry {
    /// Whether a retry repeats the exact economic input. The payer is derived
    /// from the thread, so it is checked separately against the resolved room.
    pub fn matches_request(&self, new: &NewUsageLedgerEntry) -> bool {
        self.usage_report_id == new.usage_report_id
            && self.thread_id == new.thread_id
            && self.stamp.reporter == new.reporter
            && self.stamp.claim_lease_id == new.claim_lease_id
            && self.stamp.model == new.model.trim()
            && self.stamp.tokens == new.tokens
            && self.stamp.usd_micros == new.usd_micros
            && self.stamp.price_snapshot == new.price_snapshot
            && self.turns == new.turns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_snapshot_uses_all_tiers_and_rounds_up() {
        let tokens = TokenUsage {
            input: 1_000_000,
            output: 500_000,
            cache_read: 250_000,
            cache_write: 1,
        };
        let price = PriceSnapshot {
            input_usd_micros_per_million: 10,
            output_usd_micros_per_million: 20,
            cache_read_usd_micros_per_million: 4,
            cache_write_usd_micros_per_million: 1,
        };
        assert_eq!(price.charge_usd_micros(tokens), Ok(22));
    }

    #[test]
    fn negative_or_inconsistent_economic_input_fails() {
        let mut report = NewUsageLedgerEntry {
            usage_report_id: uuid::Uuid::new_v4(),
            thread_id: ThreadId::new(),
            reporter: MemberId::new(),
            claim_lease_id: ClaimLeaseId::new(),
            model: "provider/model".into(),
            tokens: TokenUsage {
                input: 1_000_000,
                ..Default::default()
            },
            usd_micros: 10,
            price_snapshot: PriceSnapshot {
                input_usd_micros_per_million: 10,
                ..Default::default()
            },
            turns: 1,
        };
        assert_eq!(report.validate(), Ok(()));
        report.usd_micros = 9;
        assert!(report.validate().is_err());
        report.usd_micros = 10;
        report.tokens.input = -1;
        assert!(report.validate().is_err());
    }
}
