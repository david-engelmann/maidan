//! Prometheus counters for accepted usage. Recorded after the ledger
//! transaction commits, and not on an exact retry.

use maidan_types::UsageLedgerEntry;
use metrics::counter;

pub(crate) fn record(entry: &UsageLedgerEntry) {
    // No model label. The model id is caller-supplied, up to 255 bytes, and
    // there is no finite server-side catalog. A label would be one series per
    // distinct string.
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
            counter!("maidan_usage_tokens_total", "tier" => tier).increment(amount as u64);
        }
    }
    if entry.stamp.usd_micros > 0 {
        counter!("maidan_usage_usd_micros_total").increment(entry.stamp.usd_micros as u64);
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
