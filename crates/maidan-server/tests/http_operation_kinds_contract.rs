//! Every HTTP operation the router serves says whether it reads or changes.
//!
//! The request layer records any successful POST, PUT, PATCH or DELETE that
//! recorded nothing itself (`auth::run_as`), so the method is what decides
//! whether a change is guaranteed a record. `contracts/http-operation-kinds.json`
//! classifies each operation in `app.rs` as `reads` or `changes`, and says why
//! wherever the kind is not the one its method implies: a GET that writes is
//! outside that guarantee and must be named, and a POST that only reads must
//! be named so the request layer does not record it as a change.
//!
//! A HEAD is answered by the GET handler, so it shares the GET's kind.
//! `http_operation_kinds_e2e` checks the classification against what the
//! calls actually write.

use std::collections::BTreeSet;
use std::path::PathBuf;

use maidan_server::auth::READ_ONLY_OPERATIONS;

const APP: &str = include_str!("../src/app.rs");

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    method: String,
    path: String,
    kind: String,
    reason: Option<String>,
}

fn load() -> Vec<Entry> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/http-operation-kinds.json");
    serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("http operation kinds json")
}

/// The body of the call whose `(` is at `open`, skipping parentheses inside
/// string literals.
fn balanced_call(source: &str, open: usize) -> &str {
    let bytes = source.as_bytes();
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;
    for i in open..bytes.len() {
        let b = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..i];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed call beginning at byte {open}");
}

/// Every `(method, path)` registered with `.route(...)` in `app.rs`, read from
/// the source because routes outside the OpenAPI document (`/mcp`, SCIM, A2A,
/// the streams, the `/ui/api` proxies) change state too.
fn router_operations() -> BTreeSet<(String, String)> {
    let source = APP.split("#[cfg(test)]").next().unwrap_or(APP);
    let mut ops = BTreeSet::new();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find(".route(") {
        let open = cursor + found + ".route".len();
        let call = balanced_call(source, open);
        let start = call.find('"').expect("route path is a string literal") + 1;
        let end = start + call[start..].find('"').expect("route path closes");
        let path = &call[start..end];
        let mut methods = 0;
        for method in ["get", "post", "put", "patch", "delete"] {
            let needle = format!("{method}(");
            for (at, _) in call.match_indices(&needle) {
                let word_start = call[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                if word_start {
                    methods += 1;
                    assert!(
                        ops.insert((method.to_uppercase(), path.to_string())),
                        "{} {path} is registered twice",
                        method.to_uppercase()
                    );
                }
            }
        }
        assert!(methods > 0, "no HTTP method found for {path}");
        cursor = open + call.len() + 2;
    }
    ops
}

#[test]
fn every_routed_operation_is_classified_and_every_classification_is_routed() {
    let routed = router_operations();
    assert!(
        routed.len() > 350,
        "only {} operations read from app.rs; the parser has stopped seeing routes",
        routed.len()
    );
    let classified: BTreeSet<(String, String)> =
        load().into_iter().map(|e| (e.method, e.path)).collect();
    let unclassified: Vec<_> = routed.difference(&classified).collect();
    let stale: Vec<_> = classified.difference(&routed).collect();
    assert!(
        unclassified.is_empty(),
        "operations in app.rs with no entry in contracts/http-operation-kinds.json \
         (classify each as reads or changes): {unclassified:?}"
    );
    assert!(
        stale.is_empty(),
        "contracts/http-operation-kinds.json classifies operations app.rs no longer routes: {stale:?}"
    );
}

#[test]
fn the_contract_is_sorted_and_names_each_operation_once() {
    let keys: Vec<(String, String)> = load().into_iter().map(|e| (e.path, e.method)).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        keys, sorted,
        "contracts/http-operation-kinds.json must be sorted by path, then method, with no duplicates"
    );
}

#[test]
fn a_kind_its_method_does_not_imply_says_why() {
    let mut problems = Vec::new();
    for e in load() {
        let reads_by_method = match e.method.as_str() {
            "GET" | "HEAD" => true,
            "POST" | "PUT" | "PATCH" | "DELETE" => false,
            other => {
                problems.push(format!("{other} {}: unknown method", e.path));
                continue;
            }
        };
        let reads = match e.kind.as_str() {
            "reads" => true,
            "changes" => false,
            other => {
                problems.push(format!(
                    "{} {}: kind {other} is neither reads nor changes",
                    e.method, e.path
                ));
                continue;
            }
        };
        let reason = e.reason.as_deref().map(str::trim).unwrap_or_default();
        if reads != reads_by_method && reason.is_empty() {
            problems.push(format!(
                "{} {} is classified {} against its method: give the reason",
                e.method, e.path, e.kind
            ));
        }
        // A reason on an ordinary entry would make the exceptions hard to find.
        if reads == reads_by_method && e.reason.is_some() {
            problems.push(format!(
                "{} {} {} is what its method implies: drop the reason",
                e.method, e.path, e.kind
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The request layer's skip list is the contract's non-GET reads, exactly: a
/// POST classified `reads` but not skipped gets a `mutation` row claiming a
/// change, and one skipped but classified `changes` loses its record.
#[test]
fn the_request_layer_records_no_change_for_exactly_the_non_get_reads() {
    let non_get_reads: BTreeSet<String> = load()
        .into_iter()
        .filter(|e| e.kind == "reads" && e.method != "GET" && e.method != "HEAD")
        .map(|e| format!("{} {}", e.method, e.path))
        .collect();
    let skipped: BTreeSet<String> = READ_ONLY_OPERATIONS.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        skipped, non_get_reads,
        "auth::READ_ONLY_OPERATIONS must list exactly the non-GET operations \
         contracts/http-operation-kinds.json classifies as reads"
    );
    let mut sorted = READ_ONLY_OPERATIONS.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        READ_ONLY_OPERATIONS,
        sorted.as_slice(),
        "READ_ONLY_OPERATIONS is searched with binary_search, so it must be sorted"
    );
}
