//! Prometheus counters for accepted usage. Recorded after the ledger
//! transaction commits, and not on an exact retry.

use maidan_types::UsageLedgerEntry;
use metrics::counter;

pub(crate) fn record(entry: &UsageLedgerEntry) {
    // No model label. The model id is caller-supplied, up to 255 bytes, and
    // there is no finite server-side catalog. A label would be one series per
    // distinct string. The provider family is the closed set the review
    // routing uses, so cost by family costs at most one series per family.
    let family = family_label(entry);
    let tokens = entry.stamp.tokens;
    let tiers = [
        ("input", tokens.input),
        ("output", tokens.output),
        ("cache_read", tokens.cache_read),
        ("cache_write_5m", tokens.cache_write_5m),
        ("cache_write_1h", tokens.cache_write_1h),
    ];
    for (tier, amount) in tiers {
        if amount > 0 {
            counter!("maidan_usage_tokens_total", "tier" => tier, "family" => family)
                .increment(amount as u64);
        }
    }
    if entry.stamp.usd_micros > 0 {
        counter!("maidan_usage_usd_micros_total", "family" => family)
            .increment(entry.stamp.usd_micros as u64);
    }
    if let Some(standard) = entry.evidence.standard_price {
        let saved = standard
            .charge_usd_micros(tokens)
            .ok()
            .and_then(|at_standard| at_standard.checked_sub(entry.stamp.usd_micros));
        // Ingest refuses a standard price below the charge, so the saving is
        // never negative and this matches the rollup's lane_saved_usd_micros.
        if let Some(saved) = saved.filter(|saved| *saved > 0) {
            counter!("maidan_usage_lane_saved_usd_micros_total").increment(saved as u64);
        }
    }
    let Ok(uncached) = entry
        .stamp
        .price_snapshot
        .uncached_charge_usd_micros(tokens)
    else {
        return;
    };
    if uncached > 0 {
        counter!("maidan_usage_uncached_usd_micros_total").increment(uncached as u64);
    }
    match uncached.checked_sub(entry.stamp.usd_micros) {
        Some(saved) if saved > 0 => {
            counter!("maidan_usage_saved_usd_micros_total").increment(saved as u64);
        }
        Some(saved) if saved < 0 => {
            counter!("maidan_usage_write_premium_usd_micros_total").increment(saved.unsigned_abs());
        }
        _ => {}
    }
}

/// The usage record's provider family, or `other` when its provider and model
/// name none, so the label set stays closed.
fn family_label(entry: &UsageLedgerEntry) -> &'static str {
    family_of(entry.evidence.provider.as_deref(), &entry.stamp.model)
}

fn family_of(provider: Option<&str>, model: &str) -> &'static str {
    maidan_types::provider_family(provider, model).unwrap_or("other")
}

#[cfg(test)]
mod tests {
    use super::family_of;

    #[test]
    fn the_family_label_is_a_known_family_or_other() {
        assert_eq!(family_of(Some("openai"), "gpt-5.6-sol"), "openai");
        assert_eq!(family_of(None, "claude-opus-5-5"), "anthropic");
        // A caller-supplied model nobody recognizes never becomes a label.
        assert_eq!(family_of(Some("acme"), "my-private-model-7f3a"), "other");
        assert_eq!(family_of(None, ""), "other");
    }
}
