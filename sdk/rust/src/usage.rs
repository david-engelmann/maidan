//! Provider usage objects -> the ledger's `report_usage` shape.
//!
//! The ledger's `input` is uncached input on every provider, and cache writes
//! are two tiers, 5-minute and 1-hour. Providers disagree on both, so each
//! reader below says where its numbers come from. The TypeScript, Python and Go
//! SDKs and the server's own reader agree on the fixtures in
//! `sdk/usage-fixtures/`.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The ledger's token tiers. `input` is uncached input on every provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
}

/// Micro-USD per million tokens, one rate per tier, snapshotted per report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub input_usd_micros_per_million: i64,
    pub output_usd_micros_per_million: i64,
    pub cache_read_usd_micros_per_million: i64,
    pub cache_write_5m_usd_micros_per_million: i64,
    pub cache_write_1h_usd_micros_per_million: i64,
}

/// What a normalizer can tell from the response. The rest of the ledger's
/// evidence (harness, cache key, packs) is the caller's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEvidence {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_miss_reason: Option<String>,
}

/// The economic part of a `report_usage` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedUsage {
    pub model: String,
    pub tokens: TokenUsage,
    pub evidence: UsageEvidence,
}

/// Fills what a response does not say. `model` names the model when the
/// response does not (Bedrock Converse); `provider` overrides the evidence
/// provider name (a Chat Completions shape served by Azure).
#[derive(Debug, Clone, Default)]
pub struct UsageOptions {
    pub model: Option<String>,
    pub provider: Option<String>,
}

/// A usage object that cannot be read into the ledger's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

type Result<T> = std::result::Result<T, UsageError>;

fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(UsageError(message.into()))
}

/// The provider ids [`normalize_usage`] accepts.
pub const USAGE_PROVIDERS: [&str; 9] = [
    "anthropic",
    "bedrock-converse",
    "openai-responses",
    "openai-chat",
    "gemini",
    "deepseek",
    "mistral",
    "xai",
    "vllm",
];

fn at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |v, key| v.get(key))
}

fn count(value: &Value, path: &str) -> Result<i64> {
    match at(value, path) {
        None | Some(Value::Null) => Ok(0),
        Some(v) => match v.as_i64() {
            Some(n) if n >= 0 => Ok(n),
            _ => err(format!("{path} must be a non-negative integer")),
        },
    }
}

fn text(value: &Value, path: &str) -> Option<String> {
    at(value, path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn add(parts: &[i64]) -> Result<i64> {
    parts
        .iter()
        .try_fold(0_i64, |total, n| total.checked_add(*n))
        .ok_or_else(|| UsageError("token total overflow".into()))
}

/// OpenAI, Gemini, Mistral, xAI and vLLM count cached tokens inside the total.
fn uncached(total: i64, cached: &[i64]) -> Result<i64> {
    let rest = total - add(cached)?;
    if rest < 0 {
        return err("input_tokens smaller than the cache tiers it includes");
    }
    Ok(rest)
}

fn block<'a>(response: &'a Value, key: &str) -> Result<&'a Value> {
    match response.get(key) {
        Some(v @ Value::Object(_)) => Ok(v),
        _ => err(format!("response has no {key} object")),
    }
}

#[derive(Default)]
struct Reading {
    tokens: TokenUsage,
    model: Option<String>,
    service_tier: Option<String>,
    cache_miss_reason: Option<String>,
}

/// Bedrock Converse splits its writes by TTL in `cacheDetails`.
fn bedrock_details(usage: &Value) -> Result<(i64, i64)> {
    let details = match usage.get("cacheDetails") {
        None | Some(Value::Null) => return Ok((0, 0)),
        Some(Value::Array(items)) => items,
        Some(_) => return err("cacheDetails must be an array"),
    };
    let (mut five, mut hour) = (0, 0);
    for detail in details {
        if !detail.is_object() {
            return err("cacheDetails entries must be objects");
        }
        let tokens = count(detail, "inputTokens")?;
        match detail.get("ttl").and_then(Value::as_str) {
            Some("5m") => five = add(&[five, tokens])?,
            Some("1h") => hour = add(&[hour, tokens])?,
            _ => return err("cacheDetails ttl must be 5m or 1h"),
        }
    }
    Ok((five, hour))
}

fn inclusive(usage: &Value, names: [&str; 4], reasoning_is_extra: bool) -> Result<TokenUsage> {
    let [total, details, out, out_details] = names;
    let read = count(usage, &format!("{details}.cached_tokens"))?;
    let write = count(usage, &format!("{details}.cache_write_tokens"))?;
    let reasoning = if reasoning_is_extra {
        count(usage, &format!("{out_details}.reasoning_tokens"))?
    } else {
        0
    };
    Ok(TokenUsage {
        input: uncached(count(usage, total)?, &[read, write])?,
        output: add(&[count(usage, out)?, reasoning])?,
        cache_read: read,
        cache_write_5m: write,
        cache_write_1h: 0,
    })
}

const CHAT: [&str; 4] = [
    "prompt_tokens",
    "prompt_tokens_details",
    "completion_tokens",
    "completion_tokens_details",
];
const RESPONSES: [&str; 4] = [
    "input_tokens",
    "input_tokens_details",
    "output_tokens",
    "output_tokens_details",
];

fn read_anthropic(r: &Value) -> Result<Reading> {
    let u = block(r, "usage")?;
    let mut five = count(u, "cache_creation.ephemeral_5m_input_tokens")?;
    let hour = count(u, "cache_creation.ephemeral_1h_input_tokens")?;
    if five == 0 && hour == 0 {
        five = count(u, "cache_creation_input_tokens")?;
    }
    Ok(Reading {
        tokens: TokenUsage {
            input: count(u, "input_tokens")?,
            output: count(u, "output_tokens")?,
            cache_read: count(u, "cache_read_input_tokens")?,
            cache_write_5m: five,
            cache_write_1h: hour,
        },
        model: text(r, "model"),
        service_tier: text(u, "service_tier"),
        cache_miss_reason: text(r, "diagnostics.cache_miss_reason.type"),
    })
}

fn read_bedrock(r: &Value) -> Result<Reading> {
    let u = block(r, "usage")?;
    let (mut five, hour) = bedrock_details(u)?;
    if five == 0 && hour == 0 {
        five = count(u, "cacheWriteInputTokens")?;
    }
    Ok(Reading {
        tokens: TokenUsage {
            input: count(u, "inputTokens")?,
            output: count(u, "outputTokens")?,
            cache_read: count(u, "cacheReadInputTokens")?,
            cache_write_5m: five,
            cache_write_1h: hour,
        },
        ..Reading::default()
    })
}

fn read_openai(r: &Value, names: [&str; 4]) -> Result<Reading> {
    Ok(Reading {
        tokens: inclusive(block(r, "usage")?, names, false)?,
        model: text(r, "model"),
        service_tier: text(r, "service_tier"),
        ..Reading::default()
    })
}

fn read_gemini(r: &Value) -> Result<Reading> {
    let u = block(r, "usageMetadata")?;
    let read = count(u, "cachedContentTokenCount")?;
    Ok(Reading {
        tokens: TokenUsage {
            input: uncached(count(u, "promptTokenCount")?, &[read])?,
            output: add(&[
                count(u, "candidatesTokenCount")?,
                count(u, "thoughtsTokenCount")?,
            ])?,
            cache_read: read,
            ..TokenUsage::default()
        },
        model: text(r, "modelVersion"),
        service_tier: text(u, "serviceTier"),
        ..Reading::default()
    })
}

fn read_deepseek(r: &Value) -> Result<Reading> {
    let u = block(r, "usage")?;
    if matches!(u.get("prompt_cache_miss_tokens"), None | Some(Value::Null)) {
        return err("prompt_cache_miss_tokens is required");
    }
    Ok(Reading {
        tokens: TokenUsage {
            input: count(u, "prompt_cache_miss_tokens")?,
            output: count(u, "completion_tokens")?,
            cache_read: count(u, "prompt_cache_hit_tokens")?,
            ..TokenUsage::default()
        },
        model: text(r, "model"),
        ..Reading::default()
    })
}

fn read_plain_chat(r: &Value) -> Result<Reading> {
    Ok(Reading {
        tokens: inclusive(block(r, "usage")?, CHAT, false)?,
        model: text(r, "model"),
        ..Reading::default()
    })
}

fn read_xai(r: &Value) -> Result<Reading> {
    let u = block(r, "usage")?;
    let names = if u.get("input_tokens").is_some() {
        RESPONSES
    } else {
        CHAT
    };
    Ok(Reading {
        tokens: inclusive(u, names, true)?,
        model: text(r, "model"),
        ..Reading::default()
    })
}

/// Turn one provider response into the economic part of a `report_usage`
/// body: the model, the ledger's tokens, and the evidence the response carries.
pub fn normalize_usage(
    provider: &str,
    response: &Value,
    options: &UsageOptions,
) -> std::result::Result<NormalizedUsage, UsageError> {
    if !response.is_object() {
        return err("response must be a JSON object");
    }
    let (name, got) = match provider {
        "anthropic" => ("anthropic", read_anthropic(response)?),
        "bedrock-converse" => ("aws.bedrock", read_bedrock(response)?),
        "openai-responses" => ("openai", read_openai(response, RESPONSES)?),
        "openai-chat" => ("openai", read_openai(response, CHAT)?),
        "gemini" => ("gcp.gemini", read_gemini(response)?),
        "deepseek" => ("deepseek", read_deepseek(response)?),
        "mistral" => ("mistral_ai", read_plain_chat(response)?),
        "xai" => ("x_ai", read_xai(response)?),
        "vllm" => ("vllm", read_plain_chat(response)?),
        other => return err(format!("unknown provider {other}")),
    };
    let model = options
        .model
        .as_deref()
        .map(str::trim)
        .map(str::to_owned)
        .or(got.model)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            UsageError("model is required: the response names none, so pass options.model".into())
        })?;
    Ok(NormalizedUsage {
        model,
        tokens: got.tokens,
        evidence: UsageEvidence {
            provider: options.provider.clone().unwrap_or_else(|| name.to_owned()),
            service_tier: got.service_tier,
            cache_miss_reason: got.cache_miss_reason,
        },
    })
}

/// `usd_micros` as the ledger checks it: the sum of tokens times micro-USD per
/// million, divided by a million and rounded up, in integers.
pub fn usd_micros(
    tokens: &TokenUsage,
    price: &PriceSnapshot,
) -> std::result::Result<i64, UsageError> {
    let pairs = [
        (tokens.input, price.input_usd_micros_per_million),
        (tokens.output, price.output_usd_micros_per_million),
        (tokens.cache_read, price.cache_read_usd_micros_per_million),
        (
            tokens.cache_write_5m,
            price.cache_write_5m_usd_micros_per_million,
        ),
        (
            tokens.cache_write_1h,
            price.cache_write_1h_usd_micros_per_million,
        ),
    ];
    let mut sum: i128 = 0;
    for (n, rate) in pairs {
        if n < 0 || rate < 0 {
            return err("token counts and rates must be non-negative");
        }
        sum = i128::from(n)
            .checked_mul(i128::from(rate))
            .and_then(|line| sum.checked_add(line))
            .ok_or_else(|| UsageError("usd_micros overflow".into()))?;
    }
    i64::try_from((sum + 999_999) / 1_000_000).map_err(|_| UsageError("usd_micros overflow".into()))
}
