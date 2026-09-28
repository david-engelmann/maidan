//! The served OpenAPI document lints clean: the properties Redocly's
//! `recommended` ruleset checks, held by a unit test so the required `unit
//! tests` job enforces them without a Node toolchain. `scripts/openapi-lint.sh`
//! runs Redocly itself over the same document; `redocly.yaml` and
//! `.redocly.lint-ignore.yaml` carry the exceptions listed here.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::responses::PROBLEM_JSON;

const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// Operations with no client error to document: they take no credential, no
/// body and no parameters, and the rate limiter exempts them so a probe is
/// never shed.
const NO_CLIENT_ERROR: &[(&str, &str)] = &[
    ("get", "/health"),
    ("get", "/health/live"),
    ("get", "/health/ready"),
];

/// The browser login handshake answers with a redirect, never a 2xx.
const REDIRECT_ONLY: &[(&str, &str)] = &[
    ("get", "/auth/oidc/login"),
    ("get", "/auth/oidc/callback"),
    ("post", "/auth/logout"),
];

fn document() -> Value {
    serde_json::to_value(super::document()).expect("serialize the OpenAPI document")
}

fn operations(doc: &Value) -> Vec<(String, String, &Value)> {
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("paths").iter() {
        for method in METHODS {
            if let Some(op) = item.get(*method) {
                out.push(((*method).to_owned(), path.clone(), op));
            }
        }
    }
    out
}

/// A response, following a `$ref` into `#/components/responses`.
fn resolve<'a>(doc: &'a Value, response: &'a Value) -> &'a Value {
    match response.get("$ref").and_then(Value::as_str) {
        Some(reference) => doc
            .pointer(reference.trim_start_matches('#'))
            .unwrap_or_else(|| panic!("{reference} resolves to nothing")),
        None => response,
    }
}

fn statuses(op: &Value) -> BTreeSet<String> {
    op["responses"]
        .as_object()
        .map(|responses| responses.keys().cloned().collect())
        .unwrap_or_default()
}

fn listed(list: &[(&str, &str)], method: &str, path: &str) -> bool {
    list.iter().any(|(m, p)| *m == method && *p == path)
}

fn requires_credential(op: &Value) -> bool {
    op["security"].as_array().is_some_and(|reqs| {
        reqs.iter()
            .any(|r| r.as_object().is_some_and(|s| !s.is_empty()))
    })
}

#[test]
fn every_operation_has_its_own_summary() {
    let doc = document();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut problems = Vec::new();
    for (method, path, op) in operations(&doc) {
        let summary = op["summary"].as_str().unwrap_or("").trim();
        let id = format!("{} {path}", method.to_uppercase());
        if summary.is_empty() {
            problems.push(format!("{id}: no summary"));
        } else if let Some(other) = seen.insert(summary.to_owned(), id.clone()) {
            problems.push(format!("{id}: summary {summary:?} repeats {other}"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_operation_documents_a_client_error() {
    let doc = document();
    let missing: Vec<String> = operations(&doc)
        .into_iter()
        .filter(|(m, p, _)| !listed(NO_CLIENT_ERROR, m, p))
        .filter(|(_, _, op)| !statuses(op).iter().any(|s| s.starts_with('4')))
        .map(|(m, p, _)| format!("{} {p}", m.to_uppercase()))
        .collect();
    assert!(missing.is_empty(), "no 4xx response: {missing:?}");
}

#[test]
fn the_operations_excused_a_client_error_really_cannot_return_one() {
    let doc = document();
    for (method, path) in NO_CLIENT_ERROR {
        let op = &doc["paths"][*path][*method];
        assert!(op.is_object(), "{method} {path} is no longer documented");
        assert!(
            !requires_credential(op),
            "{method} {path} now takes a credential"
        );
        assert!(
            op.get("parameters").is_none(),
            "{method} {path} now takes parameters"
        );
        assert!(
            op.get("requestBody").is_none(),
            "{method} {path} now takes a body"
        );
        assert!(
            crate::rate_limit::exempt_path(path),
            "{method} {path} is no longer exempt from the rate limiter"
        );
    }
}

#[test]
fn every_operation_documents_a_success_or_its_redirect() {
    let doc = document();
    let mut problems = Vec::new();
    for (method, path, op) in operations(&doc) {
        let codes = statuses(op);
        let id = format!("{} {path}", method.to_uppercase());
        if listed(REDIRECT_ONLY, &method, &path) {
            if !codes.iter().any(|s| s.starts_with('3')) {
                problems.push(format!(
                    "{id}: listed as redirect-only but documents no 3xx"
                ));
            }
        } else if !codes.iter().any(|s| s.starts_with('2')) {
            problems.push(format!("{id}: no 2xx response"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_operation_behind_a_credential_or_the_rate_limiter_says_so() {
    let doc = document();
    let mut problems = Vec::new();
    for (method, path, op) in operations(&doc) {
        let codes = statuses(op);
        let id = format!("{} {path}", method.to_uppercase());
        if op.get("security").is_none() {
            problems.push(format!(
                "{id}: no security requirement (use `security(())` if public)"
            ));
        }
        if requires_credential(op) && !codes.contains("401") {
            problems.push(format!("{id}: takes a credential but documents no 401"));
        }
        if !crate::rate_limit::exempt_path(&path) && !codes.contains("429") {
            problems.push(format!("{id}: rate limited but documents no 429"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Whether `crate::extract` can refuse a request over this parameter: any
/// path segment (it may not percent-decode to UTF-8), and a query parameter
/// that is required or is anything but free text.
fn rejectable(parameter: &Value) -> bool {
    match parameter["in"].as_str() {
        Some("path") => true,
        Some("query") => {
            parameter["required"] == Value::Bool(true)
                || parameter["schema"] != serde_json::json!({ "type": "string" })
        }
        _ => false,
    }
}

#[test]
fn every_operation_documents_what_its_extractors_reject() {
    let doc = document();
    let mut problems = Vec::new();
    for (method, path, op) in operations(&doc) {
        let codes = statuses(op);
        let id = format!("{} {path}", method.to_uppercase());
        let body = op["requestBody"]["content"].as_object();
        let json_body = body.is_some_and(|content| content.contains_key("application/json"));
        let parameters = op["parameters"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut expect = Vec::new();
        if json_body || parameters.iter().any(rejectable) {
            expect.push("400");
        }
        if body.is_some() {
            expect.push("413");
        }
        if json_body {
            expect.push("415");
        }
        for status in expect {
            if !codes.contains(status) {
                problems.push(format!(
                    "{id}: its extractors can answer {status}; not documented"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_client_error_is_an_rfc_9457_problem() {
    let doc = document();
    let mut problems = Vec::new();
    for (method, path, op) in operations(&doc) {
        for (status, response) in op["responses"].as_object().expect("responses") {
            if !status.starts_with('4') {
                continue;
            }
            let content = &resolve(&doc, response)["content"];
            let types: Vec<&String> = content
                .as_object()
                .map(|c| c.keys().collect())
                .unwrap_or_default();
            let schema = content[PROBLEM_JSON]["schema"]["$ref"].as_str();
            if types != [PROBLEM_JSON] || schema != Some("#/components/schemas/ProblemDetails") {
                problems.push(format!(
                    "{} {path} {status}: content {types:?}, schema {schema:?}",
                    method.to_uppercase()
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn operation_ids_are_unique() {
    let doc = document();
    let mut seen = BTreeSet::new();
    let dupes: Vec<String> = operations(&doc)
        .into_iter()
        .filter_map(|(_, _, op)| op["operationId"].as_str().map(str::to_owned))
        .filter(|id| !seen.insert(id.clone()))
        .collect();
    assert!(dupes.is_empty(), "duplicate operationIds: {dupes:?}");
}

fn collect(value: &Value, refs: &mut BTreeSet<String>, nullable: &mut usize) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("$ref") {
                refs.insert(r.clone());
            }
            if map.contains_key("nullable") {
                *nullable += 1;
            }
            map.values().for_each(|v| collect(v, refs, nullable));
        }
        Value::Array(items) => items.iter().for_each(|v| collect(v, refs, nullable)),
        _ => {}
    }
}

#[test]
fn the_document_is_openapi_3_1_with_servers_and_no_unused_components() {
    let doc = document();
    assert!(
        doc["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.1")),
        "not OpenAPI 3.1: {}",
        doc["openapi"]
    );
    assert!(
        doc["servers"].as_array().is_some_and(|s| !s.is_empty()),
        "no servers declared"
    );

    let mut refs = BTreeSet::new();
    let mut nullable = 0;
    collect(&doc, &mut refs, &mut nullable);
    // 3.1 spells an optional value `type: [T, "null"]` or `oneOf` with
    // `{type: null}`; the 3.0 `nullable` keyword has no meaning here.
    assert_eq!(
        nullable, 0,
        "the 3.0 `nullable` keyword appears {nullable} times"
    );

    let mut unused = Vec::new();
    for kind in ["schemas", "responses"] {
        for name in doc["components"][kind].as_object().expect(kind).keys() {
            if !refs.contains(&format!("#/components/{kind}/{name}")) {
                unused.push(format!("{kind}/{name}"));
            }
        }
    }
    assert!(
        unused.is_empty(),
        "components nothing references: {unused:?}"
    );
}
