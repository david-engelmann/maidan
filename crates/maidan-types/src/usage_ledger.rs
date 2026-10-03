//! Accountable, retry-safe model-usage records.
//!
//! `TokenUsage::input` is uncached input on every provider. Cache writes are
//! two tiers, five minutes and one hour. A token budget counts
//! [`TokenUsage::fresh`] (uncached input, output, and both write tiers).
//! Cache reads are priced into the dollar charge and are not fresh tokens.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{ClaimLeaseId, MemberId, ThreadBudget, ThreadId, WorkspaceId};

/// Token quantities billed at independently snapshotted rates.
///
/// `input` is uncached input. A provider that reports cached tokens inside its
/// input total is normalized before it is stored; see [`token_usage_from_genai`].
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
}

impl TokenUsage {
    /// Reject a negative count. Cache reads are included: a negative read is
    /// still a bad report, it just does not count toward a token budget.
    pub fn checked(self) -> Result<(), String> {
        let values = [
            self.input,
            self.output,
            self.cache_read,
            self.cache_write_5m,
            self.cache_write_1h,
        ];
        if values.iter().any(|value| *value < 0) {
            return Err("token counts must be non-negative".into());
        }
        Ok(())
    }

    /// What the model processed fresh: uncached input, output, and cache writes.
    /// Cache reads are omitted, so a token budget is not spent by re-reading
    /// a warm prefix.
    pub fn fresh(self) -> Result<i64, String> {
        self.checked()?;
        [
            self.input,
            self.output,
            self.cache_write_5m,
            self.cache_write_1h,
        ]
        .into_iter()
        .try_fold(0_i64, i64::checked_add)
        .ok_or_else(|| "token total overflow".to_string())
    }

    /// Input-side tokens: uncached input, cache reads, and both write tiers.
    pub fn prompt(self) -> Result<i64, String> {
        self.checked()?;
        [
            self.input,
            self.cache_read,
            self.cache_write_5m,
            self.cache_write_1h,
        ]
        .into_iter()
        .try_fold(0_i64, i64::checked_add)
        .ok_or_else(|| "token total overflow".to_string())
    }
}

/// Immutable prices used for one report, in micro-USD per million tokens.
///
/// Maidan preserves this reporter-supplied evidence; it is not a live vendor
/// rate card. The write tiers are priced apart so a 5-minute write and a
/// 1-hour write are not charged as one rate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PriceSnapshot {
    pub input_usd_micros_per_million: i64,
    pub output_usd_micros_per_million: i64,
    pub cache_read_usd_micros_per_million: i64,
    pub cache_write_5m_usd_micros_per_million: i64,
    pub cache_write_1h_usd_micros_per_million: i64,
}

impl PriceSnapshot {
    /// Compute the charge in micro-USD, rounding a fractional micro-dollar up.
    pub fn charge_usd_micros(self, tokens: TokenUsage) -> Result<i64, String> {
        let rates = [
            self.input_usd_micros_per_million,
            self.output_usd_micros_per_million,
            self.cache_read_usd_micros_per_million,
            self.cache_write_5m_usd_micros_per_million,
            self.cache_write_1h_usd_micros_per_million,
        ];
        if rates.iter().any(|rate| *rate < 0) {
            return Err("price snapshot rates must be non-negative".into());
        }
        tokens.checked()?;
        let quantities = [
            tokens.input,
            tokens.output,
            tokens.cache_read,
            tokens.cache_write_5m,
            tokens.cache_write_1h,
        ];
        charge(quantities, rates)
    }

    /// What the same tokens would have cost with every input-side token at the
    /// uncached input rate and output at the output rate.
    pub fn uncached_charge_usd_micros(self, tokens: TokenUsage) -> Result<i64, String> {
        let input = tokens.prompt()?;
        self.charge_usd_micros(TokenUsage {
            input,
            output: tokens.output,
            ..TokenUsage::default()
        })
    }
}

fn charge(quantities: [i64; 5], rates: [i64; 5]) -> Result<i64, String> {
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
    i64::try_from(rounded).map_err(|_| "USD charge overflow".to_string())
}

/// Who ran the call, and what of Maidan's context it carried.
///
/// Every field is optional. Absent means the reporter did not say. Empty
/// strings are stored as absent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct UsageEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub batch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_version: Option<String>,
    /// Provider cache key or session id. Never shared across workspaces; Maidan
    /// stores what the reporter sent and does not mint one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_miss_reason: Option<String>,
    /// sha256, hex, of each Maidan pack the call used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pack_sha256: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl UsageEvidence {
    pub fn validate(&self) -> Result<(), String> {
        check_len("provider", self.provider.as_deref(), 64)?;
        check_len("service_tier", self.service_tier.as_deref(), 64)?;
        check_len("harness", self.harness.as_deref(), 64)?;
        check_len("harness_version", self.harness_version.as_deref(), 64)?;
        check_len("cache_key", self.cache_key.as_deref(), 256)?;
        check_len("cache_miss_reason", self.cache_miss_reason.as_deref(), 512)?;
        if self.pack_sha256.len() > 32 {
            return Err("at most 32 pack sha256 values".into());
        }
        for sha in &self.pack_sha256 {
            if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("pack_sha256 entries must be 64 hex characters".into());
            }
        }
        Ok(())
    }

    /// Fill only the holes. An explicit field on the request wins over one
    /// inferred from telemetry attributes.
    pub fn overlay(&mut self, explicit: &UsageEvidence) {
        if explicit.provider.is_some() {
            self.provider.clone_from(&explicit.provider);
        }
        if explicit.service_tier.is_some() {
            self.service_tier.clone_from(&explicit.service_tier);
        }
        if explicit.batch {
            self.batch = true;
        }
        if explicit.harness.is_some() {
            self.harness.clone_from(&explicit.harness);
        }
        if explicit.harness_version.is_some() {
            self.harness_version.clone_from(&explicit.harness_version);
        }
        if explicit.cache_key.is_some() {
            self.cache_key.clone_from(&explicit.cache_key);
        }
        if explicit.cache_miss_reason.is_some() {
            self.cache_miss_reason
                .clone_from(&explicit.cache_miss_reason);
        }
        if !explicit.pack_sha256.is_empty() {
            self.pack_sha256.clone_from(&explicit.pack_sha256);
        }
    }
}

fn check_len(name: &str, value: Option<&str>, max: usize) -> Result<(), String> {
    if value.is_some_and(|text| text.len() > max || text.trim().is_empty()) {
        return Err(format!("{name} must be 1..={max} bytes when set"));
    }
    Ok(())
}

fn tidy(value: Option<String>, max: usize) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > max {
        return Err(format!("evidence field longer than {max} bytes"));
    }
    Ok(Some(trimmed.to_owned()))
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
/// the resolved thread. `model` is the model id the response named.
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
    #[serde(default)]
    pub evidence: UsageEvidence,
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
            evidence: self.evidence,
        }
    }
}

/// One OpenTelemetry-style usage report. Token counts come from the GenAI
/// attributes; the price snapshot is still the reporter's, because Maidan
/// keeps no rate card. `usd_micros` is computed, not trusted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct GenAiUsageReport {
    pub usage_report_id: uuid::Uuid,
    pub claim_lease_id: ClaimLeaseId,
    pub attributes: Map<String, Value>,
    pub price_snapshot: PriceSnapshot,
    #[serde(default)]
    pub turns: i64,
    /// Wins over the same field inferred from `attributes`.
    #[serde(default)]
    pub evidence: UsageEvidence,
}

impl GenAiUsageReport {
    pub fn into_new(
        self,
        thread_id: ThreadId,
        reporter: MemberId,
    ) -> Result<NewUsageLedgerEntry, String> {
        let (tokens, mut evidence, model) = token_usage_from_genai(&self.attributes)?;
        evidence.overlay(&self.evidence);
        evidence.pack_sha256 = evidence
            .pack_sha256
            .iter()
            .map(|sha| sha.to_ascii_lowercase())
            .collect();
        let usd_micros = self.price_snapshot.charge_usd_micros(tokens)?;
        Ok(NewUsageLedgerEntry {
            usage_report_id: self.usage_report_id,
            thread_id,
            reporter,
            claim_lease_id: self.claim_lease_id,
            model,
            tokens,
            usd_micros,
            price_snapshot: self.price_snapshot,
            turns: self.turns,
            evidence,
        })
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
    #[serde(default)]
    pub evidence: UsageEvidence,
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
        self.evidence.validate()?;
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
    pub evidence: UsageEvidence,
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
            && self.evidence == new.evidence
    }
}

/// Which room a rollup covers. At most one of `thread_id` and `member_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageRollupQuery {
    pub workspace_id: WorkspaceId,
    pub thread_id: Option<ThreadId>,
    pub member_id: Option<MemberId>,
}

impl UsageRollupQuery {
    pub fn scope_name(self) -> Result<&'static str, String> {
        match (self.thread_id, self.member_id) {
            (Some(_), Some(_)) => Err("name thread_id or member_id, not both".into()),
            (Some(_), None) => Ok("thread"),
            (None, Some(_)) => Ok("member"),
            (None, None) => Ok("workspace"),
        }
    }
}

/// Spend, cache shape, and cost per completed task for one scope.
///
/// Rates are integer parts per million (1_000_000 = the whole prompt). A
/// completed task is a thread in `closed` or `archived` that has not been
/// tombstoned. `cost_per_completed_task_usd_micros` divides the spend on those
/// tasks by how many there are, truncating. It is absent when the scope has
/// no completed task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageRollup {
    pub scope: String,
    pub workspace_id: WorkspaceId,
    pub thread_id: Option<ThreadId>,
    pub member_id: Option<MemberId>,
    pub reports: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,
    pub usd_micros: i64,
    pub uncached_usd_micros: i64,
    pub saved_usd_micros: i64,
    pub hit_rate_ppm: Option<i64>,
    pub write_share_ppm: Option<i64>,
    pub completed_tasks: i64,
    pub completed_usd_micros: i64,
    pub cost_per_completed_task_usd_micros: Option<i64>,
}

/// Sums a backend query returns. The rates are computed here so both backends
/// agree.
#[derive(Debug, Clone, Copy, Default)]
pub struct UsageSums {
    pub reports: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,
    pub usd_micros: i64,
    pub uncached_usd_micros: i64,
}

impl UsageRollup {
    pub fn from_sums(
        query: UsageRollupQuery,
        sums: UsageSums,
        completed_tasks: i64,
        completed_usd_micros: i64,
    ) -> Result<Self, String> {
        let scope = query.scope_name()?.to_owned();
        if completed_tasks < 0 || completed_usd_micros < 0 {
            return Err("completed task totals must be non-negative".into());
        }
        let prompt = [
            sums.input_tokens,
            sums.cache_read_tokens,
            sums.cache_write_5m_tokens,
            sums.cache_write_1h_tokens,
        ]
        .into_iter()
        .try_fold(0_i64, i64::checked_add)
        .ok_or_else(|| "token total overflow".to_string())?;
        let saved = sums
            .uncached_usd_micros
            .checked_sub(sums.usd_micros)
            .ok_or_else(|| "saved usd overflow".to_string())?;
        let cost = if completed_tasks == 0 {
            None
        } else {
            Some(completed_usd_micros / completed_tasks)
        };
        Ok(Self {
            scope,
            workspace_id: query.workspace_id,
            thread_id: query.thread_id,
            member_id: query.member_id,
            reports: sums.reports,
            input_tokens: sums.input_tokens,
            output_tokens: sums.output_tokens,
            cache_read_tokens: sums.cache_read_tokens,
            cache_write_5m_tokens: sums.cache_write_5m_tokens,
            cache_write_1h_tokens: sums.cache_write_1h_tokens,
            usd_micros: sums.usd_micros,
            uncached_usd_micros: sums.uncached_usd_micros,
            saved_usd_micros: saved,
            hit_rate_ppm: ppm(sums.cache_read_tokens, prompt)?,
            write_share_ppm: ppm(
                sums.cache_write_5m_tokens
                    .checked_add(sums.cache_write_1h_tokens)
                    .ok_or_else(|| "token total overflow".to_string())?,
                prompt,
            )?,
            completed_tasks,
            completed_usd_micros,
            cost_per_completed_task_usd_micros: cost,
        })
    }
}

fn ppm(part: i64, whole: i64) -> Result<Option<i64>, String> {
    if whole == 0 {
        return Ok(None);
    }
    let value = i128::from(part)
        .checked_mul(1_000_000)
        .ok_or_else(|| "rate overflow".to_string())?
        / i128::from(whole);
    i64::try_from(value)
        .map(Some)
        .map_err(|_| "rate overflow".to_string())
}

/// Normalize a GenAI usage attribute map into the ledger's shape.
///
/// `input` in the result is uncached. Providers that include cache reads and
/// writes in their input total (OpenAI, Azure, Gemini, Mistral, xAI, vLLM,
/// and the OpenTelemetry names for those) have those subtracted. Anthropic,
/// Bedrock (`bedrock`, `aws.bedrock`), and DeepSeek already report uncached
/// input. `gcp.vertex_ai` is not classified: Vertex serves both shapes.
/// A write total that is not split by TTL is recorded on the 5-minute tier,
/// which is Anthropic's default and the 1.25x write the other providers publish.
pub fn token_usage_from_genai(
    attrs: &Map<String, Value>,
) -> Result<(TokenUsage, UsageEvidence, String), String> {
    let provider = attr_string(
        attrs,
        &["gen_ai.provider.name", "gen_ai.system", "provider"],
    )?;
    let model = attr_string(
        attrs,
        &["gen_ai.response.model", "gen_ai.request.model", "model"],
    )?
    .filter(|text| !text.is_empty())
    .ok_or_else(|| "gen_ai response model is required".to_string())?;
    if model.len() > 255 {
        return Err("model must contain 1..=255 bytes".into());
    }

    let mut cache_read = attr_i64(
        attrs,
        &[
            "gen_ai.usage.cache_read.input_tokens",
            "cache_read_input_tokens",
            "gen_ai.usage.cached_tokens",
            "cached_tokens",
            "prompt_tokens_details.cached_tokens",
            "prompt_cache_hit_tokens",
            "cachedContentTokenCount",
        ],
    )?
    .unwrap_or(0);
    let mut write_5m = attr_i64(
        attrs,
        &[
            "gen_ai.usage.cache_creation.ephemeral_5m_input_tokens",
            "cache_creation.ephemeral_5m_input_tokens",
            "ephemeral_5m_input_tokens",
        ],
    )?
    .unwrap_or(0);
    let write_1h = attr_i64(
        attrs,
        &[
            "gen_ai.usage.cache_creation.ephemeral_1h_input_tokens",
            "cache_creation.ephemeral_1h_input_tokens",
            "ephemeral_1h_input_tokens",
        ],
    )?
    .unwrap_or(0);
    if write_5m == 0 && write_1h == 0 {
        write_5m = attr_i64(
            attrs,
            &[
                "gen_ai.usage.cache_creation.input_tokens",
                "cache_creation_input_tokens",
                "cache_write_input_tokens",
                "cacheWriteInputTokens",
                "prompt_tokens_details.cache_write_tokens",
                "gen_ai.usage.cache_write.input_tokens",
            ],
        )?
        .unwrap_or(0);
    }

    let raw_input = attr_i64(
        attrs,
        &[
            "gen_ai.usage.input_tokens",
            "input_tokens",
            "inputTokens",
            "prompt_tokens",
            "promptTokenCount",
        ],
    )?
    .unwrap_or(0);
    let miss = attr_i64(
        attrs,
        &[
            "prompt_cache_miss_tokens",
            "gen_ai.usage.prompt_cache_miss_tokens",
        ],
    )?;
    let output = attr_i64(
        attrs,
        &[
            "gen_ai.usage.output_tokens",
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "candidatesTokenCount",
        ],
    )?
    .unwrap_or(0);

    let provider_key = provider.as_deref().map(str::to_ascii_lowercase);
    let inclusive = matches!(
        provider_key.as_deref(),
        Some(
            "openai"
                | "azure"
                | "azure.ai.openai"
                | "azure.ai.inference"
                | "openrouter"
                | "gemini"
                | "google"
                | "gcp.gemini"
                | "gcp.gen_ai"
                | "mistral"
                | "mistral_ai"
                | "xai"
                | "x_ai"
                | "vllm"
        )
    );
    let exclusive = matches!(
        provider_key.as_deref(),
        Some("anthropic" | "bedrock" | "aws.bedrock" | "amazon" | "deepseek")
    );
    let input = if let Some(miss) = miss {
        cache_read = attr_i64(
            attrs,
            &[
                "prompt_cache_hit_tokens",
                "gen_ai.usage.cache_read.input_tokens",
            ],
        )?
        .unwrap_or(cache_read);
        miss
    } else if inclusive {
        raw_input
            .checked_sub(cache_read)
            .and_then(|n| n.checked_sub(write_5m))
            .and_then(|n| n.checked_sub(write_1h))
            .ok_or_else(|| "input_tokens smaller than the cache tiers it includes".to_string())?
    } else if exclusive {
        raw_input
    } else {
        let writes = write_5m
            .checked_add(write_1h)
            .ok_or_else(|| "token total overflow".to_string())?;
        let cached = cache_read
            .checked_add(writes)
            .ok_or_else(|| "token total overflow".to_string())?;
        if raw_input >= cached {
            raw_input - cached
        } else {
            raw_input
        }
    };

    let tokens = TokenUsage {
        input,
        output,
        cache_read,
        cache_write_5m: write_5m,
        cache_write_1h: write_1h,
    };
    tokens.checked()?;

    let batch_attr = attrs
        .get("gen_ai.request.batch")
        .or_else(|| attrs.get("batch"));
    let batch = match batch_attr {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => {
            matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
        }
        _ => attr_string(attrs, &["gen_ai.operation.name"])?
            .is_some_and(|name| name.eq_ignore_ascii_case("batch")),
    };
    let packs = pack_shas(attrs.get("maidan.pack.sha256"))?;
    let evidence = UsageEvidence {
        provider: tidy(provider, 64)?,
        service_tier: tidy(
            attr_string(attrs, &["gen_ai.request.service_tier", "service_tier"])?,
            64,
        )?,
        batch,
        harness: tidy(attr_string(attrs, &["gen_ai.agent.name", "harness"])?, 64)?,
        harness_version: tidy(
            attr_string(attrs, &["gen_ai.agent.version", "harness_version"])?,
            64,
        )?,
        cache_key: tidy(
            attr_string(
                attrs,
                &[
                    "gen_ai.conversation.id",
                    "session.id",
                    "cache_key",
                    "prompt_cache_key",
                ],
            )?,
            256,
        )?,
        cache_miss_reason: tidy(
            attr_string(
                attrs,
                &["gen_ai.usage.cache_miss_reason", "cache_miss_reason"],
            )?,
            512,
        )?,
        pack_sha256: packs,
    };
    evidence.validate()?;
    Ok((tokens, evidence, model))
}

fn pack_shas(value: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let raw: Vec<String> = match value {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "maidan.pack.sha256 entries must be strings".to_string())
            })
            .collect::<Result<_, _>>()?,
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect(),
        Value::Null => Vec::new(),
        _ => return Err("maidan.pack.sha256 must be a string or an array".into()),
    };
    Ok(raw
        .into_iter()
        .map(|sha| sha.to_ascii_lowercase())
        .collect())
}

fn attr_string(attrs: &Map<String, Value>, keys: &[&str]) -> Result<Option<String>, String> {
    for key in keys {
        if let Some(value) = attrs.get(*key) {
            match value {
                Value::String(text) => return Ok(Some(text.trim().to_owned())),
                Value::Null => continue,
                _ => return Err(format!("{key} must be a string")),
            }
        }
    }
    Ok(None)
}

fn attr_i64(attrs: &Map<String, Value>, keys: &[&str]) -> Result<Option<i64>, String> {
    for key in keys {
        if let Some(value) = attrs.get(*key) {
            let parsed = match value {
                Value::Number(number) => number
                    .as_i64()
                    .ok_or_else(|| format!("{key} is not an integer"))?,
                Value::String(text) => text
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| format!("{key} is not an integer"))?,
                Value::Null => continue,
                _ => return Err(format!("{key} is not an integer")),
            };
            if parsed < 0 {
                return Err(format!("{key} must be non-negative"));
            }
            return Ok(Some(parsed));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_snapshot_uses_both_write_tiers_and_rounds_up() {
        let tokens = TokenUsage {
            input: 1_000_000,
            output: 500_000,
            cache_read: 250_000,
            cache_write_5m: 1,
            cache_write_1h: 1,
        };
        let price = PriceSnapshot {
            input_usd_micros_per_million: 10,
            output_usd_micros_per_million: 20,
            cache_read_usd_micros_per_million: 4,
            cache_write_5m_usd_micros_per_million: 1,
            cache_write_1h_usd_micros_per_million: 1,
        };
        assert_eq!(price.charge_usd_micros(tokens), Ok(22));
    }

    #[test]
    fn fresh_tokens_omit_cache_reads() {
        let tokens = TokenUsage {
            input: 10,
            output: 4,
            cache_read: 1_000,
            cache_write_5m: 2,
            cache_write_1h: 3,
        };
        assert_eq!(tokens.fresh(), Ok(19));
        assert_eq!(tokens.prompt(), Ok(1_015));
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
            evidence: UsageEvidence::default(),
        };
        assert_eq!(report.validate(), Ok(()));
        report.usd_micros = 9;
        assert!(report.validate().is_err());
        report.usd_micros = 10;
        report.tokens.input = -1;
        assert!(report.validate().is_err());
    }

    #[test]
    fn anthropic_attributes_keep_input_uncached_and_split_writes() {
        let attrs = serde_json::json!({
            "gen_ai.system": "anthropic",
            "gen_ai.response.model": "claude-sonnet",
            "gen_ai.usage.input_tokens": 100,
            "gen_ai.usage.output_tokens": 7,
            "gen_ai.usage.cache_read.input_tokens": 300,
            "gen_ai.usage.cache_creation.ephemeral_5m_input_tokens": 50,
            "gen_ai.usage.cache_creation.ephemeral_1h_input_tokens": 20,
            "gen_ai.agent.name": "claude-code",
            "gen_ai.agent.version": "2.1.0",
            "maidan.pack.sha256": ["ab".repeat(32)]
        });
        let map = attrs.as_object().unwrap();
        let (tokens, evidence, model) = token_usage_from_genai(map).unwrap();
        assert_eq!(model, "claude-sonnet");
        assert_eq!(tokens.input, 100);
        assert_eq!(tokens.cache_read, 300);
        assert_eq!(tokens.cache_write_5m, 50);
        assert_eq!(tokens.cache_write_1h, 20);
        assert_eq!(tokens.output, 7);
        assert_eq!(evidence.provider.as_deref(), Some("anthropic"));
        assert_eq!(evidence.harness.as_deref(), Some("claude-code"));
        assert_eq!(evidence.pack_sha256.len(), 1);
    }

    #[test]
    fn openai_attributes_subtract_cached_tokens_from_input() {
        let attrs = serde_json::json!({
            "gen_ai.provider.name": "openai",
            "gen_ai.response.model": "gpt",
            "gen_ai.usage.input_tokens": 1_000,
            "gen_ai.usage.output_tokens": 1,
            "gen_ai.usage.cached_tokens": 400,
            "prompt_tokens_details.cache_write_tokens": 100
        });
        let (tokens, _, _) = token_usage_from_genai(attrs.as_object().unwrap()).unwrap();
        assert_eq!(tokens.input, 500);
        assert_eq!(tokens.cache_read, 400);
        assert_eq!(tokens.cache_write_5m, 100);
        assert_eq!(tokens.cache_write_1h, 0);
    }

    #[test]
    fn deepseek_miss_and_hit_are_uncached_and_read() {
        let attrs = serde_json::json!({
            "gen_ai.system": "deepseek",
            "gen_ai.response.model": "deepseek-chat",
            "prompt_cache_miss_tokens": 12,
            "prompt_cache_hit_tokens": 80,
            "gen_ai.usage.output_tokens": 3
        });
        let (tokens, _, _) = token_usage_from_genai(attrs.as_object().unwrap()).unwrap();
        assert_eq!(tokens.input, 12);
        assert_eq!(tokens.cache_read, 80);
        assert_eq!(tokens.fresh().unwrap(), 15);
    }

    #[test]
    fn otel_provider_names_follow_the_short_name_input_rule() {
        // Exclusive and larger than the cache read. The unclassified fallback
        // would subtract here and undercount.
        for provider in [
            "anthropic",
            "bedrock",
            "aws.bedrock",
            "AWS.Bedrock",
            "amazon",
            "deepseek",
        ] {
            let attrs = serde_json::json!({
                "gen_ai.provider.name": provider,
                "gen_ai.response.model": "m",
                "gen_ai.usage.input_tokens": 1_000,
                "gen_ai.usage.cache_read.input_tokens": 400,
                "gen_ai.usage.output_tokens": 1
            });
            let (tokens, _, _) = token_usage_from_genai(attrs.as_object().unwrap()).unwrap();
            assert_eq!(tokens.input, 1_000, "{provider}");
            assert_eq!(tokens.cache_read, 400, "{provider}");
        }
        // Inclusive and smaller than the cache read. Subtraction goes negative
        // and the count check rejects it. The fallback would keep the raw total.
        for provider in [
            "openai",
            "azure",
            "azure.ai.openai",
            "azure.ai.inference",
            "openrouter",
            "gemini",
            "google",
            "gcp.gemini",
            "gcp.gen_ai",
            "mistral",
            "mistral_ai",
            "xai",
            "x_ai",
            "vllm",
        ] {
            let attrs = serde_json::json!({
                "gen_ai.provider.name": provider,
                "gen_ai.response.model": "m",
                "gen_ai.usage.input_tokens": 100,
                "gen_ai.usage.cache_read.input_tokens": 400,
                "gen_ai.usage.output_tokens": 1
            });
            let err = token_usage_from_genai(attrs.as_object().unwrap()).unwrap_err();
            assert!(err.contains("non-negative"), "{provider}: {err}");
        }
    }

    #[test]
    fn vertex_ai_stays_unclassified() {
        let large = serde_json::json!({
            "gen_ai.provider.name": "gcp.vertex_ai",
            "gen_ai.response.model": "m",
            "gen_ai.usage.input_tokens": 1_000,
            "gen_ai.usage.cache_read.input_tokens": 400,
            "gen_ai.usage.output_tokens": 1
        });
        let (tokens, _, _) = token_usage_from_genai(large.as_object().unwrap()).unwrap();
        assert_eq!(tokens.input, 600);
        let small = serde_json::json!({
            "gen_ai.provider.name": "gcp.vertex_ai",
            "gen_ai.response.model": "m",
            "gen_ai.usage.input_tokens": 100,
            "gen_ai.usage.cache_read.input_tokens": 400,
            "gen_ai.usage.output_tokens": 1
        });
        let (tokens, _, _) = token_usage_from_genai(small.as_object().unwrap()).unwrap();
        assert_eq!(tokens.input, 100);
    }

    #[test]
    fn rollup_rates_and_cost_per_completed_task() {
        let query = UsageRollupQuery {
            workspace_id: WorkspaceId::new(),
            thread_id: None,
            member_id: None,
        };
        let rollup = UsageRollup::from_sums(
            query,
            UsageSums {
                reports: 2,
                input_tokens: 100,
                output_tokens: 0,
                cache_read_tokens: 300,
                cache_write_5m_tokens: 50,
                cache_write_1h_tokens: 50,
                usd_micros: 293,
                uncached_usd_micros: 500,
            },
            2,
            200,
        )
        .unwrap();
        assert_eq!(rollup.hit_rate_ppm, Some(600_000));
        assert_eq!(rollup.write_share_ppm, Some(200_000));
        assert_eq!(rollup.saved_usd_micros, 207);
        assert_eq!(rollup.cost_per_completed_task_usd_micros, Some(100));
        assert_eq!(rollup.scope, "workspace");
    }

    fn attrs(value: serde_json::Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn sha(c: char) -> String {
        c.to_string().repeat(64)
    }

    #[test]
    fn prompt_tokens_include_cache_reads_and_writes() {
        let tokens = TokenUsage {
            input: 1,
            output: 100,
            cache_read: 2,
            cache_write_5m: 3,
            cache_write_1h: 4,
        };
        assert_eq!(tokens.prompt(), Ok(10));
        assert_eq!(tokens.fresh(), Ok(108));
    }

    #[test]
    fn token_sums_report_overflow_and_negative_counts() {
        let big = TokenUsage {
            input: i64::MAX,
            output: 1,
            ..TokenUsage::default()
        };
        assert!(big.fresh().unwrap_err().contains("overflow"));
        let big_prompt = TokenUsage {
            input: i64::MAX,
            cache_read: 1,
            ..TokenUsage::default()
        };
        assert!(big_prompt.prompt().unwrap_err().contains("overflow"));
        let negative = TokenUsage {
            cache_read: -1,
            ..TokenUsage::default()
        };
        assert!(negative.checked().is_err());
        assert!(negative.prompt().is_err());
    }

    #[test]
    fn uncached_charge_prices_every_prompt_token_at_the_input_rate() {
        let price = PriceSnapshot {
            input_usd_micros_per_million: 1_000_000,
            output_usd_micros_per_million: 2_000_000,
            cache_read_usd_micros_per_million: 100_000,
            cache_write_5m_usd_micros_per_million: 1_250_000,
            cache_write_1h_usd_micros_per_million: 2_000_000,
        };
        let tokens = TokenUsage {
            input: 10,
            output: 5,
            cache_read: 20,
            cache_write_5m: 3,
            cache_write_1h: 2,
        };
        assert_eq!(price.uncached_charge_usd_micros(tokens), Ok(45));
        assert!(price.charge_usd_micros(tokens).unwrap() < 45);
    }

    #[test]
    fn price_snapshot_rejects_negative_rates_and_overflow() {
        let negative = PriceSnapshot {
            output_usd_micros_per_million: -1,
            ..PriceSnapshot::default()
        };
        assert!(negative
            .charge_usd_micros(TokenUsage::default())
            .unwrap_err()
            .contains("non-negative"));
        let steep = PriceSnapshot {
            input_usd_micros_per_million: i64::MAX,
            ..PriceSnapshot::default()
        };
        let huge = TokenUsage {
            input: i64::MAX,
            ..TokenUsage::default()
        };
        assert!(steep
            .charge_usd_micros(huge)
            .unwrap_err()
            .contains("overflow"));
    }

    #[test]
    fn evidence_validation_rejects_long_blank_and_malformed_fields() {
        assert!(UsageEvidence::default().validate().is_ok());
        let long = UsageEvidence {
            provider: Some("x".repeat(65)),
            ..UsageEvidence::default()
        };
        assert!(long.validate().is_err());
        let blank = UsageEvidence {
            cache_key: Some("   ".into()),
            ..UsageEvidence::default()
        };
        assert!(blank.validate().is_err());
        let too_many = UsageEvidence {
            pack_sha256: vec![sha('a'); 33],
            ..UsageEvidence::default()
        };
        assert!(too_many.validate().unwrap_err().contains("at most 32"));
        let bad_hex = UsageEvidence {
            pack_sha256: vec!["zz".repeat(32)],
            ..UsageEvidence::default()
        };
        assert!(bad_hex.validate().unwrap_err().contains("64 hex"));
        let short = UsageEvidence {
            pack_sha256: vec!["ab".into()],
            ..UsageEvidence::default()
        };
        assert!(short.validate().is_err());
    }

    #[test]
    fn evidence_overlay_lets_explicit_fields_win_and_keeps_the_rest() {
        let mut inferred = UsageEvidence {
            provider: Some("openai".into()),
            harness: Some("inferred".into()),
            pack_sha256: vec![sha('a')],
            ..UsageEvidence::default()
        };
        let explicit = UsageEvidence {
            provider: Some("azure".into()),
            service_tier: Some("flex".into()),
            batch: true,
            harness_version: Some("2".into()),
            cache_key: Some("key".into()),
            cache_miss_reason: Some("ttl".into()),
            pack_sha256: vec![sha('b')],
            ..UsageEvidence::default()
        };
        inferred.overlay(&explicit);
        assert_eq!(inferred.provider.as_deref(), Some("azure"));
        assert_eq!(inferred.harness.as_deref(), Some("inferred"));
        assert_eq!(inferred.service_tier.as_deref(), Some("flex"));
        assert!(inferred.batch);
        assert_eq!(inferred.harness_version.as_deref(), Some("2"));
        assert_eq!(inferred.cache_key.as_deref(), Some("key"));
        assert_eq!(inferred.cache_miss_reason.as_deref(), Some("ttl"));
        assert_eq!(inferred.pack_sha256, vec![sha('b')]);

        let mut kept = inferred.clone();
        kept.overlay(&UsageEvidence::default());
        assert_eq!(kept, inferred);
    }

    #[test]
    fn evidence_serde_omits_empty_fields_and_round_trips() {
        let empty = serde_json::to_value(UsageEvidence::default()).unwrap();
        assert_eq!(empty, serde_json::json!({}));
        let full = UsageEvidence {
            provider: Some("anthropic".into()),
            batch: true,
            pack_sha256: vec![sha('c')],
            ..UsageEvidence::default()
        };
        let back: UsageEvidence =
            serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
        assert_eq!(back, full);
        assert!(serde_json::from_value::<UsageEvidence>(serde_json::json!({"bogus": 1})).is_err());
    }

    #[test]
    fn rollup_scope_names_follow_which_id_is_set() {
        let workspace_id = WorkspaceId::new();
        let thread = UsageRollupQuery {
            workspace_id,
            thread_id: Some(ThreadId::new()),
            member_id: None,
        };
        assert_eq!(thread.scope_name(), Ok("thread"));
        let member = UsageRollupQuery {
            workspace_id,
            thread_id: None,
            member_id: Some(MemberId::new()),
        };
        assert_eq!(member.scope_name(), Ok("member"));
        let both = UsageRollupQuery {
            thread_id: Some(ThreadId::new()),
            ..member
        };
        assert!(both.scope_name().is_err());
    }

    #[test]
    fn rollup_without_prompt_or_completed_tasks_has_no_rates() {
        let query = UsageRollupQuery {
            workspace_id: WorkspaceId::new(),
            thread_id: None,
            member_id: None,
        };
        let rollup = UsageRollup::from_sums(query, UsageSums::default(), 0, 0).unwrap();
        assert_eq!(rollup.hit_rate_ppm, None);
        assert_eq!(rollup.write_share_ppm, None);
        assert_eq!(rollup.cost_per_completed_task_usd_micros, None);
        assert!(UsageRollup::from_sums(query, UsageSums::default(), -1, 0).is_err());
        assert!(UsageRollup::from_sums(query, UsageSums::default(), 0, -1).is_err());
        let overflow = UsageSums {
            input_tokens: i64::MAX,
            cache_read_tokens: 1,
            ..UsageSums::default()
        };
        assert!(UsageRollup::from_sums(query, overflow, 0, 0).is_err());
        let rate_overflow = UsageSums {
            input_tokens: i64::MAX / 2,
            cache_read_tokens: i64::MAX / 2,
            ..UsageSums::default()
        };
        assert!(UsageRollup::from_sums(query, rate_overflow, 0, 0).is_ok());
    }

    #[test]
    fn unknown_provider_input_is_reduced_only_when_it_covers_the_cache() {
        let covers = attrs(serde_json::json!({
            "gen_ai.response.model": "m",
            "gen_ai.usage.input_tokens": 100,
            "gen_ai.usage.cache_read.input_tokens": 30,
            "gen_ai.usage.cache_creation.input_tokens": 20,
        }));
        let (tokens, _, _) = token_usage_from_genai(&covers).unwrap();
        assert_eq!((tokens.input, tokens.cache_write_5m), (50, 20));
        let smaller = attrs(serde_json::json!({
            "gen_ai.response.model": "m",
            "gen_ai.usage.input_tokens": 10,
            "gen_ai.usage.cache_read.input_tokens": 30,
        }));
        let (tokens, _, _) = token_usage_from_genai(&smaller).unwrap();
        assert_eq!(tokens.input, 10);
    }

    #[test]
    fn inclusive_provider_input_smaller_than_its_cache_tiers_is_rejected() {
        let bad = attrs(serde_json::json!({
            "gen_ai.provider.name": "OpenAI",
            "gen_ai.response.model": "m",
            "gen_ai.usage.input_tokens": 5,
            "gen_ai.usage.cache_read.input_tokens": 30,
        }));
        assert!(token_usage_from_genai(&bad).is_err());
    }

    #[test]
    fn genai_attributes_require_a_model_and_well_typed_values() {
        assert!(token_usage_from_genai(&Map::new())
            .unwrap_err()
            .contains("model"));
        let blank = attrs(serde_json::json!({"model": "  "}));
        assert!(token_usage_from_genai(&blank).is_err());
        let long = attrs(serde_json::json!({"model": "m".repeat(256)}));
        assert!(token_usage_from_genai(&long).unwrap_err().contains("255"));
        let bad_type = attrs(serde_json::json!({"model": 7}));
        assert!(token_usage_from_genai(&bad_type)
            .unwrap_err()
            .contains("must be a string"));
        for (value, needle) in [
            (serde_json::json!(-1), "non-negative"),
            (serde_json::json!(1.5), "not an integer"),
            (serde_json::json!("abc"), "not an integer"),
            (serde_json::json!(true), "not an integer"),
        ] {
            let a = attrs(serde_json::json!({"model": "m", "output_tokens": value}));
            assert!(token_usage_from_genai(&a).unwrap_err().contains(needle));
        }
        let text =
            attrs(serde_json::json!({"model": "m", "output_tokens": " 12 ", "input_tokens": null}));
        let (tokens, _, _) = token_usage_from_genai(&text).unwrap();
        assert_eq!((tokens.output, tokens.input), (12, 0));
    }

    #[test]
    fn genai_evidence_reads_batch_tier_harness_cache_and_packs() {
        let a = attrs(serde_json::json!({
            "gen_ai.provider.name": "anthropic",
            "gen_ai.response.model": "m",
            "gen_ai.request.batch": "Yes",
            "gen_ai.request.service_tier": " flex ",
            "gen_ai.agent.name": "goose",
            "gen_ai.agent.version": "1.2",
            "session.id": "s1",
            "cache_miss_reason": "ttl",
            "maidan.pack.sha256": format!("{}, {}", sha('A'), sha('b')),
        }));
        let (_, evidence, model) = token_usage_from_genai(&a).unwrap();
        assert_eq!(model, "m");
        assert!(evidence.batch);
        assert_eq!(evidence.service_tier.as_deref(), Some("flex"));
        assert_eq!(evidence.harness.as_deref(), Some("goose"));
        assert_eq!(evidence.harness_version.as_deref(), Some("1.2"));
        assert_eq!(evidence.cache_key.as_deref(), Some("s1"));
        assert_eq!(evidence.cache_miss_reason.as_deref(), Some("ttl"));
        assert_eq!(evidence.pack_sha256, vec![sha('a'), sha('b')]);

        let by_op = attrs(serde_json::json!({"model": "m", "gen_ai.operation.name": "BATCH"}));
        assert!(token_usage_from_genai(&by_op).unwrap().1.batch);
        let flag = attrs(serde_json::json!({"model": "m", "batch": true}));
        assert!(token_usage_from_genai(&flag).unwrap().1.batch);
        let none = attrs(serde_json::json!({"model": "m", "batch": "no"}));
        assert!(!token_usage_from_genai(&none).unwrap().1.batch);
    }

    #[test]
    fn pack_sha_attribute_accepts_arrays_and_rejects_other_shapes() {
        let array = attrs(serde_json::json!({"model": "m", "maidan.pack.sha256": [sha('d')]}));
        assert_eq!(
            token_usage_from_genai(&array).unwrap().1.pack_sha256,
            vec![sha('d')]
        );
        let null = attrs(serde_json::json!({"model": "m", "maidan.pack.sha256": null}));
        assert!(token_usage_from_genai(&null)
            .unwrap()
            .1
            .pack_sha256
            .is_empty());
        let non_string = attrs(serde_json::json!({"model": "m", "maidan.pack.sha256": [1]}));
        assert!(token_usage_from_genai(&non_string).is_err());
        let number = attrs(serde_json::json!({"model": "m", "maidan.pack.sha256": 1}));
        assert!(token_usage_from_genai(&number).is_err());
        let malformed = attrs(serde_json::json!({"model": "m", "maidan.pack.sha256": "abc"}));
        assert!(token_usage_from_genai(&malformed).is_err());
    }

    #[test]
    fn genai_report_computes_the_charge_and_lets_explicit_evidence_win() {
        let report = GenAiUsageReport {
            usage_report_id: uuid::Uuid::new_v4(),
            claim_lease_id: ClaimLeaseId::new(),
            attributes: attrs(serde_json::json!({
                "gen_ai.provider.name": "anthropic",
                "gen_ai.response.model": "m",
                "gen_ai.usage.input_tokens": 1_000_000,
            })),
            price_snapshot: PriceSnapshot {
                input_usd_micros_per_million: 3,
                ..PriceSnapshot::default()
            },
            turns: 2,
            evidence: UsageEvidence {
                provider: Some("override".into()),
                pack_sha256: vec![sha('E')],
                ..UsageEvidence::default()
            },
        };
        let entry = report.into_new(ThreadId::new(), MemberId::new()).unwrap();
        assert_eq!(entry.usd_micros, 3);
        assert_eq!(entry.turns, 2);
        assert_eq!(entry.model, "m");
        assert_eq!(entry.evidence.provider.as_deref(), Some("override"));
        assert_eq!(entry.evidence.pack_sha256, vec![sha('e')]);
        assert!(entry.validate().is_ok());
    }

    #[test]
    fn accounted_request_becomes_an_entry_that_validates_its_charge() {
        let price = PriceSnapshot {
            input_usd_micros_per_million: 1_000_000,
            ..PriceSnapshot::default()
        };
        let request = AccountedUsageRequest {
            usage_report_id: uuid::Uuid::new_v4(),
            claim_lease_id: ClaimLeaseId::new(),
            model: "m".into(),
            tokens: TokenUsage {
                input: 5,
                ..TokenUsage::default()
            },
            usd_micros: 5,
            price_snapshot: price,
            turns: 1,
            evidence: UsageEvidence::default(),
        };
        let mut entry = request.into_new(ThreadId::new(), MemberId::new());
        assert!(entry.validate().is_ok());
        entry.usd_micros = 6;
        assert!(entry.validate().unwrap_err().contains("usd_micros"));
        entry.usd_micros = -1;
        assert!(entry.validate().is_err());
        entry.usd_micros = 5;
        entry.evidence.cache_key = Some("x".repeat(257));
        assert!(entry.validate().is_err());
    }
}
