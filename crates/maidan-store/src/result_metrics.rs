//! Prometheus counters for stored thread results, so a swarm's verdict mix and
//! outcomes can be read per harness without reading results. Recorded after
//! the write commits.
//!
//! Every label is a closed set: a caller-supplied `result_kind` or `status`
//! that names nothing on these lists reads as `other`, and the verdict is one
//! of a fixed few words, so no free text, id or model name becomes a series.

use metrics::counter;
use serde_json::Value;

/// Harnesses a `result_kind` can name before its first `.`
/// (`codex.redteam/1` is the codex harness).
const HARNESSES: &[&str] = &["codex", "claude", "grok", "gemini", "pi", "maidan"];

/// What the result is, after the harness (`codex.redteam/1` is a redteam).
const KINDS: &[&str] = &["redteam", "security", "review", "plan", "build", "fix"];

const STATUSES: &[&str] = &[
    "reviewed",
    "done",
    "failed",
    "blocked",
    "partial",
    "cancelled",
];

const VERDICTS: &[&str] = &["pass", "fix", "merge", "block", "reject"];

pub(crate) fn record(result: &Value) {
    let labels = labels(result);
    counter!(
        "maidan_thread_results_total",
        "harness" => labels.harness,
        "kind" => labels.kind,
        "status" => labels.status,
        "verdict" => labels.verdict,
    )
    .increment(1);
}

#[derive(Debug, PartialEq, Eq)]
struct Labels {
    harness: &'static str,
    kind: &'static str,
    status: &'static str,
    verdict: &'static str,
}

fn labels(result: &Value) -> Labels {
    let text = |key: &str| result.get(key).and_then(Value::as_str);
    let (harness, kind) = match text("result_kind") {
        None => ("none", "none"),
        Some(kind) => {
            let (head, rest) = kind.split_once('.').unwrap_or((kind, ""));
            let rest = rest.split('/').next().unwrap_or_default();
            (closed(HARNESSES, head), closed(KINDS, rest))
        }
    };
    let status = text("status").map_or("none", |status| closed(STATUSES, status));
    Labels {
        harness,
        kind,
        status,
        verdict: text("rendered").map_or("none", verdict),
    }
}

fn closed(set: &[&'static str], value: &str) -> &'static str {
    set.iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(value))
        .unwrap_or("other")
}

/// The word after the first `VERDICT` in a rendered report's opening lines
/// (`VERDICT: FIX`, `VERDICT FIX @ <sha>`), or `none`.
fn verdict(rendered: &str) -> &'static str {
    rendered
        .lines()
        .take(20)
        .find_map(|line| {
            let after = line.trim().strip_prefix("VERDICT")?;
            let word = after
                .trim_start_matches(|c: char| c == ':' || c.is_whitespace())
                .split(|c: char| !c.is_ascii_alphabetic())
                .next()?;
            Some(closed(VERDICTS, word))
        })
        .unwrap_or("none")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_red_team_result_reads_as_harness_kind_status_and_verdict() {
        let result = json!({
            "result_kind": "codex.redteam/1",
            "status": "reviewed",
            "rendered": "Red-team by Codex at abc\n\nVERDICT: FIX\n\nHEAD abc",
        });
        assert_eq!(
            labels(&result),
            Labels {
                harness: "codex",
                kind: "redteam",
                status: "reviewed",
                verdict: "fix"
            }
        );
    }

    #[test]
    fn free_text_never_becomes_a_label() {
        let result = json!({
            "result_kind": "acme-secret-project.thing/9",
            "status": "shipped it to alice@example.com",
            "rendered": "VERDICT: ship-it-now",
        });
        assert_eq!(
            labels(&result),
            Labels {
                harness: "other",
                kind: "other",
                status: "other",
                verdict: "other"
            }
        );
        assert_eq!(
            labels(&json!({"answer": 42})),
            Labels {
                harness: "none",
                kind: "none",
                status: "none",
                verdict: "none"
            }
        );
    }
}
