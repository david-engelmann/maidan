//! Request handlers never spawn.
//!
//! Attribution — who acted, for whom, under which grant — is a task-local scope
//! set once per request (`maidan_store::attribution`). A task spawned from a
//! handler leaves that scope, so anything it writes is recorded as background
//! work: an event with no `attribution`, and no `mutation` row for the change.
//! Nothing would fail; the record would just be wrong.
//!
//! No handler spawns today. This makes that a checked property rather than a
//! convention. A handler that genuinely needs background work should hand it
//! to a worker that records its own principal, or carry the attribution into
//! the task explicitly — and then be exempted here, by name, with the reason.

use std::path::{Path, PathBuf};

/// Handler files allowed to spawn, each with why its task writes no record.
const EXEMPT: &[(&str, &str)] = &[];

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

#[test]
fn no_request_handler_spawns_out_of_its_attribution_scope() {
    let server = Path::new(env!("CARGO_MANIFEST_DIR"));
    let handler_dirs = [
        server.join("src/routes"),
        server.join("../maidan-mcp/src/tools"),
    ];
    let mut files = Vec::new();
    for dir in &handler_dirs {
        rust_files(dir, &mut files);
    }
    assert!(files.len() > 20, "found only {} handler files", files.len());

    let mut offenders = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        if EXEMPT.iter().any(|(exempt, _)| *exempt == name) {
            continue;
        }
        let source = std::fs::read_to_string(&file).unwrap();
        // Test modules may spawn freely; they are not request handlers.
        let handler_code = source.split("#[cfg(test)]").next().unwrap_or_default();
        for (line_no, line) in handler_code.lines().enumerate() {
            if line.contains("tokio::spawn") || line.contains("task::spawn") {
                offenders.push(format!("{}:{}", file.display(), line_no + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a request handler spawns a task, which runs outside the request's \
         attribution scope and records its writes as nobody's: {offenders:?}"
    );
}
