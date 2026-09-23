//! Keep the no-build `/ui` console's HTTP paths tied to the public contract.
//!
//! There are deliberately no copied route tables here. The test derives all
//! three views from their owners: path templates from the embedded UI,
//! session-proxy routes from `app.rs`, and public operations from `ApiDoc`.

use std::collections::{BTreeMap, BTreeSet};

use maidan_server::openapi::ApiDoc;
use utoipa::openapi::path::PathItemType;
use utoipa::OpenApi;

const HTML: &str = include_str!("../static/index.html");
const APP: &str = include_str!("../src/app.rs");

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RouteKey {
    method: String,
    path: String,
}

fn method_name(kind: &PathItemType) -> &'static str {
    match kind {
        PathItemType::Get => "GET",
        PathItemType::Post => "POST",
        PathItemType::Put => "PUT",
        PathItemType::Patch => "PATCH",
        PathItemType::Delete => "DELETE",
        PathItemType::Options => "OPTIONS",
        PathItemType::Head => "HEAD",
        PathItemType::Trace => "TRACE",
        PathItemType::Connect => "CONNECT",
    }
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
            item.operations.keys().map(move |kind| RouteKey {
                method: method_name(kind).to_owned(),
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

fn script() -> &'static str {
    let start = HTML.find("<script>").expect("script") + "<script>".len();
    let end = HTML[start..].find("</script>").expect("script end") + start;
    &HTML[start..end]
}

/// Extract JS string and template literals. Route candidates are selected
/// later, after normalization, so ordinary UI copy remains harmless.
fn string_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
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
            .all(|(actual, expected)| *actual == "{}" || *actual == expected)
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
fn ui_path_templates_resolve_to_openapi() {
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
    for literal in string_literals(script()) {
        let Some(path) = normalize_ui_template(&literal) else {
            continue;
        };
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
        "UI route-like templates that do not resolve to OpenAPI: {missing:#?}"
    );
    assert!(
        candidates.len() >= 35,
        "route census unexpectedly shrank to {}; parser may no longer see the UI fetch templates",
        candidates.len()
    );
}

#[test]
fn ui_session_path_templates_resolve_to_the_proxy() {
    let proxy_paths = session_proxy_routes()
        .into_iter()
        .map(|route| route.path)
        .collect::<BTreeSet<_>>();

    // Every literal passed directly to either proxy helper must be mounted by
    // the session router. Variable suffixes remain covered by the full UI
    // route census above and by the proxy-vs-OpenAPI method contract.
    for helper in ["uiReadPath(", "apiWritePath("] {
        let mut cursor = 0;
        while let Some(found) = script()[cursor..].find(helper) {
            let open = cursor + found + helper.len() - 1;
            let call = balanced_call(script(), open);
            let argument = call.trim_start();
            if let Some(quote) = argument.as_bytes().first().copied() {
                if matches!(quote, b'\'' | b'"' | b'`') {
                    let literals = string_literals(argument);
                    if let Some(path) = literals.first().and_then(|v| normalize_ui_template(v)) {
                        assert!(
                            proxy_paths.iter().any(|proxy| path_matches(&path, proxy)),
                            "{helper} path {path:?} is not mounted by the session proxy"
                        );
                    }
                }
            }
            cursor = open + call.len() + 2;
        }
    }
}

#[test]
fn ui_inline_fetch_methods_resolve_to_the_session_proxy() {
    let proxy = session_proxy_routes();
    let source = script();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find("fetch(") {
        let open = cursor + found + "fetch".len();
        let call = balanced_call(source, open);
        for helper in ["uiReadPath(", "apiWritePath("] {
            let Some(found_helper) = call.find(helper) else {
                continue;
            };
            let helper_open = found_helper + helper.len() - 1;
            let helper_call = balanced_call(call, helper_open);
            let argument = helper_call.trim_start();
            let Some(quote) = argument.as_bytes().first().copied() else {
                continue;
            };
            if !matches!(quote, b'\'' | b'"' | b'`') {
                // Indirect suffixes are included in the all-template census;
                // this check handles calls whose path and method are local.
                continue;
            }
            let path = normalize_ui_template(
                string_literals(argument)
                    .first()
                    .expect("quoted helper argument"),
            )
            .expect("helper path");
            let methods = if helper == "uiReadPath(" {
                vec!["GET"]
            } else if let Some(method_at) = call.find("method:") {
                let method_expr = call[method_at + "method:".len()..]
                    .split_once(',')
                    .map_or(&call[method_at + "method:".len()..], |(value, _)| value);
                let methods = ["GET", "POST", "PUT", "PATCH", "DELETE"]
                    .into_iter()
                    .filter(|method| method_expr.contains(&format!("\"{method}\"")))
                    .collect::<Vec<_>>();
                assert!(
                    !methods.is_empty(),
                    "could not derive inline method for apiWritePath({path})"
                );
                methods
            } else {
                vec!["GET"]
            };
            for method in methods {
                assert!(
                    proxy.iter().any(|route| {
                        route.method == method && path_matches(&path, &route.path)
                    }),
                    "inline UI fetch {method} {path} is not mounted with that method by the session proxy"
                );
            }
        }
        cursor = open + call.len() + 2;
    }
}
