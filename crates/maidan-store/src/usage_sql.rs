//! The rollup's per-row charge in SQL, without a 64-bit overflow (#1595).
//!
//! A row's charge is `ceil(Σ tokens × rate / 1_000_000)`. The products can
//! exceed `i64` even when the charge itself fits (validation checks it in
//! `i128`), so the SQL never forms them. Each rate splits into
//! `rate = q·10⁶ + r` and each token count into `tokens = t·10⁶ + b`, so
//! `tokens × rate = 10⁶·(tokens·q + t·r) + b·r`. `tokens·q` and `t·r` are at
//! most `tokens × rate / 10⁶`, so they fit whenever the charge fits, and
//! `b·r < 10¹²`. The result is the same integer on Postgres and SQLite.

/// SQL for `ceil(Σ quantity × rate / 1_000_000)` over `(quantity, rate)`
/// expression pairs of non-negative integers.
pub(crate) fn ceil_micros(terms: &[(&str, &str)]) -> String {
    let whole: Vec<String> = terms
        .iter()
        .map(|(q, r)| format!("({q}) * (({r}) / 1000000) + (({q}) / 1000000) * (({r}) % 1000000)"))
        .collect();
    let rest: Vec<String> = terms
        .iter()
        .map(|(q, r)| format!("(({q}) % 1000000) * (({r}) % 1000000)"))
        .collect();
    format!(
        "({}) + (({}) + 999999) / 1000000",
        whole.join(" + "),
        rest.join(" + ")
    )
}

/// The rollup's `uncached_usd_micros` per row: every input-side token at the
/// input rate, output at the output rate.
pub(crate) fn uncached_row() -> String {
    ceil_micros(&[
        (
            "input_tokens + cache_read_tokens + cache_write_5m_tokens + cache_write_1h_tokens",
            "input_price",
        ),
        ("output_tokens", "output_price"),
    ])
}

/// The rollup's `lane_standard_usd_micros` per row, at the standard rates.
pub(crate) fn lane_standard_row() -> String {
    ceil_micros(&[
        ("input_tokens", "standard_input_price"),
        ("output_tokens", "standard_output_price"),
        ("cache_read_tokens", "standard_cache_read_price"),
        ("cache_write_5m_tokens", "standard_cache_write_5m_price"),
        ("cache_write_1h_tokens", "standard_cache_write_1h_price"),
    ])
}
