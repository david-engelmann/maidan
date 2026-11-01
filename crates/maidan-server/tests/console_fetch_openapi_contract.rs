//! Keep the console's HTTP paths tied to the public contract: every
//! route-like path template in `web/console/src` resolves to an OpenAPI
//! operation, and every session-proxy route under `/ui/api` is one too.
//! Ported from the classic board's `ui_fetch_openapi_contract`; the sources
//! are walked from disk so a new module is covered without editing this test.
//! A segment built from an expression (`${what}-follows`) counts as a
//! parameter.

use std::collections::{BTreeMap, BTreeSet};

use maidan_server::openapi::ApiDoc;
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::OpenApi;

const APP: &str = include_str!("../src/app.rs");

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RouteKey {
    method: String,
    path: String,
}

fn operations(item: &PathItem) -> impl Iterator<Item = (&'static str, &Operation)> {
    [
        ("GET", &item.get),
        ("POST", &item.post),
        ("PUT", &item.put),
        ("PATCH", &item.patch),
        ("DELETE", &item.delete),
        ("OPTIONS", &item.options),
        ("HEAD", &item.head),
        ("TRACE", &item.trace),
    ]
    .into_iter()
    .filter_map(|(method, op)| op.as_ref().map(|op| (method, op)))
}

/// Normalize both Axum (`:id`) and OpenAPI (`{id}`) parameters to `{}`. UI
/// expressions are normalized by `normalize_ui_template` below.
fn normalize_contract_path(path: &str) -> String {
    path.split('/')
        .map(|part| {
            if part.starts_with(':') || (part.starts_with('{') && part.ends_with('}')) {
                "{}"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn openapi_routes() -> BTreeSet<RouteKey> {
    ApiDoc::openapi()
        .paths
        .paths
        .iter()
        .flat_map(|(path, item)| {
            operations(item).map(move |(method, _)| RouteKey {
                method: method.to_owned(),
                path: normalize_contract_path(path),
            })
        })
        .collect()
}

fn balanced_call(source: &str, open: usize) -> &str {
    let bytes = source.as_bytes();
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for i in open..bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == q {
                quote = None;
            }
            continue;
        }
        match b {
            b'\'' | b'"' | b'`' => quote = Some(b),
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

fn first_quoted(call: &str) -> &str {
    let start = call.find('"').expect("route path starts with a string") + 1;
    let end = call[start..]
        .find('"')
        .map(|n| start + n)
        .expect("route path string closes");
    &call[start..end]
}

fn session_proxy_routes() -> BTreeSet<RouteKey> {
    let start = APP.find("let ui_api_read =").expect("ui read router");
    let end = APP[start..]
        .find("let ui_api =")
        .map(|n| start + n)
        .expect("merged ui router");
    let source = &APP[start..end];
    let mut routes = BTreeSet::new();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find(".route(") {
        let open = cursor + found + ".route".len();
        let call = balanced_call(source, open);
        let path = first_quoted(call)
            .strip_prefix("/ui/api")
            .expect("session proxy route is mounted below /ui/api");
        let path = normalize_contract_path(path);
        let mut found_method = false;
        for (needle, method) in [
            ("get(", "GET"),
            ("post(", "POST"),
            ("put(", "PUT"),
            ("patch(", "PATCH"),
            ("delete(", "DELETE"),
        ] {
            if call.contains(needle) {
                found_method = true;
                routes.insert(RouteKey {
                    method: method.to_owned(),
                    path: path.clone(),
                });
            }
        }
        assert!(found_method, "no HTTP method found for {path}");
        cursor = open + call.len() + 2;
    }
    routes
}

fn console_sources() -> String {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read console dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "ts" || ext == "tsx")
            {
                out.push(path);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/console/src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();
    assert!(
        files.len() >= 10,
        "web/console/src has only {} modules",
        files.len()
    );
    files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("read console module"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract JS string and template literals. Route candidates are selected
/// later, after normalization, so ordinary UI copy remains harmless.
fn string_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Comments are not strings. A quote inside one must not hide a route
        // template that moved with its function.
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        let quote = bytes[i];
        if !matches!(quote, b'\'' | b'"' | b'`') {
            i += 1;
            continue;
        }
        let start = i + 1;
        i = start;
        let mut escaped = false;
        while i < bytes.len() {
            if escaped {
                escaped = false;
            } else if bytes[i] == b'\\' {
                escaped = true;
            } else if bytes[i] == quote {
                out.push(source[start..i].to_owned());
                i += 1;
                break;
            }
            i += 1;
        }
    }
    out
}

/// Turn `/threads/${threadId}/messages?limit=50` into
/// `/threads/{}/messages`. Expressions are path parameters; query strings do
/// not affect OpenAPI path matching.
fn normalize_ui_template(value: &str) -> Option<String> {
    if !value.starts_with('/') || value.starts_with("//") {
        return None;
    }
    let path = value.split(['?', '#']).next().unwrap_or(value);
    let bytes = path.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"${") {
            let mut depth = 1_u32;
            i += 2;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            if depth != 0 {
                return None;
            }
            out.push_str("{}");
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Some(out)
}

fn path_matches(template: &str, contract: &str) -> bool {
    let template = template.split('/').collect::<Vec<_>>();
    let contract = contract.split('/').collect::<Vec<_>>();
    template.len() == contract.len()
        && template
            .iter()
            .zip(contract)
            .all(|(actual, expected)| actual.contains("{}") || *actual == expected)
}

#[test]
fn session_proxy_methods_and_paths_are_openapi_operations() {
    let public = openapi_routes();
    let missing = session_proxy_routes()
        .difference(&public)
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "session proxy routes missing the same method/path in OpenAPI: {missing:#?}"
    );
}

#[test]
fn console_path_templates_resolve_to_openapi() {
    let public = openapi_routes();
    let public_paths = public
        .iter()
        .map(|route| route.path.as_str())
        .collect::<BTreeSet<_>>();
    let roots = public_paths
        .iter()
        .filter_map(|path| path.split('/').nth(1))
        .filter(|part| !part.is_empty() && *part != "{}")
        .collect::<BTreeSet<_>>();

    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for literal in string_literals(&console_sources()) {
        let Some(path) = normalize_ui_template(&literal) else {
            continue;
        };
        // A literal already under the session proxy names the same suffix.
        let path = path
            .strip_prefix("/ui/api")
            .map_or(path.clone(), str::to_owned);
        // A bare root (`/members`) is a prefix the code joins onto, and the
        // page's own `/ui` paths are assets, not API calls.
        if path.split('/').filter(|part| !part.is_empty()).count() < 2
            || path == "/ui"
            || path.starts_with("/ui/")
        {
            continue;
        }
        let Some(root) = path.split('/').nth(1) else {
            continue;
        };
        if roots.contains(root) {
            candidates.entry(path).or_default().insert(literal);
        }
    }

    let missing = candidates
        .iter()
        .filter(|(path, _)| {
            !public_paths
                .iter()
                .any(|contract| path_matches(path, contract))
        })
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "console route-like templates that do not resolve to OpenAPI: {missing:#?}"
    );
    assert!(
        candidates.len() >= 60,
        "route census unexpectedly shrank to {}; the parser may no longer see the console's fetch templates",
        candidates.len()
    );
}

#[test]
fn a_built_segment_is_a_parameter() {
    assert!(path_matches(
        "/members/{}/{}-follows",
        "/members/{}/channel-follows"
    ));
    assert!(!path_matches("/members/{}/x", "/members/{}/y"));
    assert_eq!(
        normalize_ui_template("/threads/${id}/messages?limit=50").as_deref(),
        Some("/threads/{}/messages")
    );
}
