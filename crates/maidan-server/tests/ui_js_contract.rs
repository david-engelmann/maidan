//! Guard against "function called but never defined" in the `/ui` console JS
//! (Cluster 133). The `/ui` is vanilla HTML/JS with no browser in CI, so a
//! reference-to-undefined-function bug (which is exactly what broke the write
//! path — `apiWritePath`/`requireAuthForWrite` were called but never defined)
//! otherwise sails through `cargo test`. This is a dependency-free static check:
//! every *bare* call `ident(` must resolve to a local definition, a function
//! parameter, or a known JS/DOM global.

const HTML: &str = include_str!("../static/index.html");

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

fn script(html: &str) -> &str {
    let start = html.find("<script>").expect("a <script> block") + "<script>".len();
    let end = html[start..].find("</script>").expect("a </script>") + start;
    &html[start..end]
}

/// The identifier ending at byte `idx` (exclusive), scanning left.
fn ident_ending_at(s: &str, idx: usize) -> Option<(usize, &str)> {
    let bytes = s.as_bytes();
    let mut i = idx;
    while i > 0 && is_ident(bytes[i - 1] as char) {
        i -= 1;
    }
    if i == idx {
        None
    } else {
        Some((i, &s[i..idx]))
    }
}

/// The identifier starting at byte `idx`, scanning right.
fn ident_starting_at(s: &str, idx: usize) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut j = idx;
    while j < s.len() && is_ident(bytes[j] as char) {
        j += 1;
    }
    if j == idx {
        None
    } else {
        Some(&s[idx..j])
    }
}

/// Names introduced by `function NAME`, `const/let/var NAME`.
fn collect_defined(s: &str, out: &mut std::collections::HashSet<String>) {
    for kw in ["function ", "const ", "let ", "var "] {
        let mut from = 0;
        while let Some(rel) = s[from..].find(kw) {
            let after = from + rel + kw.len();
            // whole-word keyword (preceded by non-ident)
            let pre_ok = from + rel == 0 || !is_ident(s.as_bytes()[from + rel - 1] as char);
            if pre_ok {
                if let Some(name) = ident_starting_at(s, after) {
                    out.insert(name.to_string());
                }
            }
            from = after;
        }
    }
}

/// Best-effort function-parameter names: identifiers inside a `(...)` that is
/// immediately followed by `=>`, plus a single `x =>` param. Conservative — it
/// over-collects (treats any such ident as defined), which only *weakens* the
/// check, never produces a false failure.
fn collect_params(s: &str, out: &mut std::collections::HashSet<String>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(rel) = s[i..].find("=>") {
        let arrow = i + rel;
        // skip whitespace left of =>
        let mut k = arrow;
        while k > 0 && (bytes[k - 1] as char).is_whitespace() {
            k -= 1;
        }
        if k > 0 && bytes[k - 1] == b')' {
            // (... ) => : collect idents inside the matching paren group
            let close = k - 1;
            let mut depth = 1i32;
            let mut p = close;
            while p > 0 && depth > 0 {
                p -= 1;
                match bytes[p] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    _ => {}
                }
            }
            let inner = &s[p + 1..close];
            for tok in inner.split(|c: char| !is_ident(c)) {
                if !tok.is_empty() && !tok.chars().next().unwrap().is_ascii_digit() {
                    out.insert(tok.to_string());
                }
            }
        } else if let Some((_, name)) = ident_ending_at(s, k) {
            // x => : single bare param
            out.insert(name.to_string());
        }
        i = arrow + 2;
    }
}

/// JS keywords + globals that can legitimately appear as a bare `ident(`.
const ALLOWED: &[&str] = &[
    // keywords that precede `(`
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "return",
    "function",
    "typeof",
    "await",
    "do",
    // JS globals / builtins
    "fetch",
    "alert",
    "confirm",
    "prompt",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "btoa",
    "atob",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Object",
    "Promise",
    "Map",
    "Set",
    "Date",
    "Error",
    "RegExp",
    "Symbol",
    "JSON",
    "Math",
    "structuredClone",
    "queueMicrotask",
    "requestAnimationFrame",
    "WebSocket",
    "URL",
    "URLSearchParams",
    "Blob",
    "FormData",
    "TextEncoder",
    "TextDecoder",
    "Uint8Array",
];

#[test]
fn ui_js_has_no_undefined_bare_function_calls() {
    let s = script(HTML);
    let mut known = std::collections::HashSet::new();
    collect_defined(s, &mut known);
    collect_params(s, &mut known);
    for a in ALLOWED {
        known.insert((*a).to_string());
    }

    let bytes = s.as_bytes();
    let mut unresolved: Vec<String> = Vec::new();
    for (idx, b) in bytes.iter().enumerate() {
        if *b != b'(' {
            continue;
        }
        let Some((start, name)) = ident_ending_at(s, idx) else {
            continue;
        };
        // skip method calls (`foo.bar(`) and property access — preceded by `.`
        if start > 0 && bytes[start - 1] == b'.' {
            continue;
        }
        // skip pure-numeric (won't happen for idents) already handled by ident scan
        if !known.contains(name) && !unresolved.contains(&name.to_string()) {
            unresolved.push(name.to_string());
        }
    }

    assert!(
        unresolved.is_empty(),
        "/ui index.html calls these as functions but they are neither defined, \
         a parameter, nor a known global — likely a typo or a removed helper \
         (CI has no browser to catch this at runtime): {unresolved:?}"
    );
}

/// Cluster 153: the live thread view wires WS event frames into the message
/// list. No browser in CI, so guard the wiring statically — the helper must be
/// defined, invoked, and driven by the thread-content kind set + the open-thread
/// predicate in the WS handler.
#[test]
fn ui_js_wires_live_thread_refresh() {
    let s = script(HTML);
    assert!(
        s.contains("function scheduleLiveRefresh("),
        "scheduleLiveRefresh must be defined"
    );
    assert!(
        s.contains("scheduleLiveRefresh()"),
        "scheduleLiveRefresh must be invoked (dead helper otherwise)"
    );
    assert!(
        s.contains("liveFrameTargetsOpenThread(v)"),
        "the WS handler must gate the refresh on the open thread"
    );
    for kind in [
        "message_posted",
        "message_edited",
        "reaction_added",
        "message_pinned",
    ] {
        assert!(s.contains(kind), "THREAD_CONTENT_KINDS must include {kind}");
    }
}

/// Cluster 353.1: the Session tab's capability card. No browser in the required
/// jobs, so guard the wiring statically — the loader must be defined, wired to the
/// tab switch, and read the real grant from `/me`.
#[test]
fn ui_js_wires_session_capability_card() {
    let s = script(HTML);
    assert!(
        s.contains("function loadSession("),
        "loadSession must be defined"
    );
    assert!(
        s.contains("uiReadPath(\"/me\")"),
        "the card must read the real grant from GET /me"
    );
    assert!(
        s.contains("known_capabilities"),
        "\"can't\" must be computed from the known-capability vocabulary"
    );
    assert!(
        HTML.contains("data-tab=\"session\""),
        "the Session tab button must exist"
    );
    assert!(
        HTML.contains("id=\"panel-session\""),
        "the Session panel must exist"
    );
    assert!(
        s.contains("\"session\") loadSession()"),
        "the tab switch must invoke loadSession()"
    );
}
