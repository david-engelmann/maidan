//! The cache helpers against the shared cases in `sdk/cache-fixtures/`, which
//! the other SDKs read too (no server needed).

use std::path::PathBuf;

use maidan::{cache_key, cache_key_fields, cached_prefix, gateway_session, BootPrefix, CacheError};
use serde_json::Value;

fn cases() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../cache-fixtures/cases.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn each(section: &str, check: impl Fn(&Value, Result<Value, CacheError>)) {
    for case in cases()[section].as_array().unwrap() {
        let got = match section {
            "cached_prefix" => cached_prefix(
                case["provider"].as_str().unwrap(),
                case["text"].as_str().unwrap(),
                case["ttl"].as_str(),
            ),
            "cache_key_fields" => cache_key_fields(case["provider"].as_str().unwrap(), "K")
                .map(|f| serde_json::to_value(f).unwrap()),
            "gateway_session" => gateway_session(
                case["gateway"].as_str().unwrap(),
                case["thread_id"].as_str().unwrap(),
                None,
                None,
            )
            .map(|f| serde_json::to_value(f).unwrap()),
            other => panic!("no section {other}"),
        };
        check(case, got);
    }
}

fn expect(case: &Value, got: Result<Value, CacheError>) {
    match case.get("expected_error").and_then(Value::as_str) {
        Some(needle) => {
            let err = got.expect_err(&case.to_string());
            assert!(err.0.contains(needle), "{case}: {err}");
        }
        None => assert_eq!(got.unwrap(), case["expected"], "{case}"),
    }
}

#[test]
fn the_boot_prefix_keeps_the_served_bytes_and_hashes_them() {
    let boot = &cases()["boot"];
    let got = BootPrefix::new(boot["text"].as_str().unwrap().to_owned());
    assert_eq!(serde_json::to_value(got).unwrap(), *boot);
}

#[test]
fn the_prefix_is_placed_with_each_providers_breakpoint() {
    each("cached_prefix", expect);
}

#[test]
fn a_cache_key_is_per_workspace_and_group() {
    let all = cases();
    let keys = all["cache_key"].as_array().unwrap();
    for case in keys {
        let got = cache_key(
            case["workspace_id"].as_str().unwrap(),
            case["group"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(got, case["expected"].as_str().unwrap());
    }
    assert_eq!(keys[0]["group"], keys[1]["group"]);
    assert_ne!(
        keys[0]["expected"], keys[1]["expected"],
        "two workspaces never share a key"
    );
    assert!(cache_key("", "g").is_err());
}

#[test]
fn the_cache_key_goes_where_each_provider_reads_it() {
    each("cache_key_fields", expect);
}

#[test]
fn the_thread_id_is_each_gateways_session_id() {
    each("gateway_session", expect);
}
