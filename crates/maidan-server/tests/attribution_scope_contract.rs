//! Code that serves a request never spawns out of its attribution scope.
//!
//! Attribution — who acted, for whom, under which grant — is a task-local scope
//! set once per request (`maidan_store::attribution`). A task spawned with
//! `tokio::spawn` leaves that scope, so anything it writes is recorded as
//! background work: an event with no `attribution`, and no `mutation` row for
//! the change. Nothing would fail; the record would just be wrong.
//!
//! This scanned only `routes/` and the MCP tools, and said no handler spawned.
//! The WebSocket subscription, the SSE streams, A2A subscribe and push, and the
//! reindex job all did. Every module of the server and of MCP is scanned now,
//! so a new one is covered without being listed. Outside the background
//! modules below, a task is spawned with `maidan_store::attribution::spawn`,
//! which carries the scope into it (and is `tokio::spawn` outside a request).
//! A `spawn_blocking` closure cannot await the store, so it is not counted.

use std::path::{Path, PathBuf};

/// Modules that never run inside a request, each with why.
const BACKGROUND: &[(&str, &str)] = &[
    ("main.rs", "boot: starts the workers and the server"),
    (
        "automation_worker.rs",
        "a worker loop; each delivery records its own principal",
    ),
    ("federation_worker.rs", "a worker loop pulling from peers"),
    ("fsm_hook_worker.rs", "a worker loop running FSM hooks"),
    (
        "webhook_worker.rs",
        "a worker loop delivering webhooks from the bus",
    ),
    (
        "notification_router.rs",
        "consumes the bus after the request that published an event has ended",
    ),
    (
        "content_keys.rs",
        "the key rewrap one boot runs once, before serving",
    ),
    (
        "presence.rs",
        "the hub's listener, heartbeat and publisher, started at boot; they write nothing to the store",
    ),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The index just past the item that starts at `from`: its `;`, or the brace
/// that closes its body. Skips string, raw string and char literals, so a `{`
/// inside one does not count.
fn end_of_item(src: &[u8], from: usize) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < src.len() {
        match src[i] {
            b'"' => {
                i += 1;
                while i < src.len() && src[i] != b'"' {
                    if src[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'r' if src.get(i + 1).is_some_and(|b| *b == b'"' || *b == b'#') => {
                let hashes = src[i + 1..].iter().take_while(|b| **b == b'#').count();
                if src.get(i + 1 + hashes) == Some(&b'"') {
                    let close: Vec<u8> = std::iter::once(b'"')
                        .chain(std::iter::repeat_n(b'#', hashes))
                        .collect();
                    i += 2 + hashes;
                    while i < src.len() && !src[i..].starts_with(&close) {
                        i += 1;
                    }
                    i += close.len() - 1;
                }
            }
            b'\'' if src.get(i + 1) == Some(&b'\\') && src.get(i + 3) == Some(&b'\'') => i += 3,
            b'\'' if src.get(i + 2) == Some(&b'\'') => i += 2,
            b';' if depth == 0 => return i + 1,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    src.len()
}

/// `source` with every `#[cfg(test)]` item blanked out, line breaks kept so
/// line numbers still match. Test code may spawn freely.
fn without_tests(source: &str) -> String {
    let mut out = source.as_bytes().to_vec();
    let mut from = 0;
    while let Some(found) = source[from..].find("#[cfg(test)]") {
        let start = from + found;
        let end = end_of_item(source.as_bytes(), start + "#[cfg(test)]".len());
        for b in &mut out[start..end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        from = end;
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn blanking_test_items_keeps_the_code_after_them() {
    let src = "fn a() {}\n#[cfg(test)]\nmod t { fn x() { let _ = \"}\"; let _ = '{'; let _ = '\\''; } }\nfn b() { tokio::spawn(x); }\n";
    let kept = without_tests(src);
    assert!(kept.contains("fn a()"));
    assert!(!kept.contains("mod t"));
    assert!(kept.contains("fn b() { tokio::spawn(x); }"));
    assert_eq!(kept.lines().count(), src.lines().count());
}

#[test]
fn request_serving_code_spawns_only_inside_its_attribution_scope() {
    let server = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots = [server.join("src"), server.join("../maidan-mcp/src")];
    let mut files = Vec::new();
    for dir in &roots {
        rust_files(dir, &mut files);
    }
    assert!(files.len() > 100, "found only {} source files", files.len());

    for (name, _) in BACKGROUND {
        assert!(
            files
                .iter()
                .any(|f| f.file_name().is_some_and(|n| n == *name)),
            "{name} is listed as a background module but no longer exists"
        );
    }

    let mut offenders = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        if BACKGROUND.iter().any(|(background, _)| *background == name) {
            continue;
        }
        let source = std::fs::read_to_string(&file).unwrap();
        for (line_no, line) in without_tests(&source).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("tokio::spawn(") || line.contains("task::spawn(") {
                offenders.push(format!("{}:{}", file.display(), line_no + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "code that serves requests spawns with tokio::spawn, which leaves the \
         request's attribution scope and records its writes as nobody's; use \
         maidan_store::attribution::spawn, or list a module that never serves \
         a request in BACKGROUND with the reason: {offenders:?}"
    );
}
