//! The board page preloads its whole static module graph, so the browser
//! fetches every module in parallel instead of discovering them one import
//! level at a time. The `<link rel="modulepreload">` block in
//! `static/index.html` is derived from the `import` statements reachable from
//! `main.js`; rerun with `MAIDAN_WRITE_SNAPSHOTS=1` (or `make regen`) after
//! changing an import.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const BEGIN: &str = "<!-- modulepreload:begin (generated; make regen) -->";
const END: &str = "<!-- modulepreload:end -->";

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("static/ui")
}

/// Relative module specifiers (`./x.js`) named by static or dynamic imports.
fn imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for quote in ['"', '\''] {
        for marker in [
            format!("from {quote}./"),
            format!("import({quote}./"),
            format!("import {quote}./"),
        ] {
            let mut rest = source;
            while let Some(at) = rest.find(&marker) {
                let tail = &rest[at + marker.len()..];
                if let Some(end) = tail.find(quote) {
                    out.push(tail[..end].to_string());
                }
                rest = tail;
            }
        }
    }
    out
}

/// Every module reachable from `main.js`, excluding the entry itself.
fn module_graph() -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec!["main.js".to_string()];
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let source = std::fs::read_to_string(ui_dir().join(&name))
            .unwrap_or_else(|e| panic!("{name} is imported but unreadable: {e}"));
        stack.extend(imports(&source));
    }
    seen.remove("main.js");
    seen
}

fn expected_block(graph: &BTreeSet<String>) -> String {
    let mut block = format!("    {BEGIN}\n");
    for name in graph {
        block.push_str(&format!(
            "    <link rel=\"modulepreload\" href=\"/ui/static/{name}\" />\n"
        ));
    }
    block.push_str(&format!("    {END}\n"));
    block
}

#[test]
fn the_board_page_preloads_exactly_the_modules_main_js_imports() {
    let graph = module_graph();
    assert!(
        graph.len() > 5,
        "the module graph looks unparsed: {graph:?}"
    );
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("static/index.html");
    let html = std::fs::read_to_string(&path).unwrap();
    let start = html
        .find(BEGIN)
        .map(|i| html[..i].rfind('\n').map_or(0, |n| n + 1));
    let stop = html.find(END).map(|i| i + END.len() + 1);
    let (Some(start), Some(stop)) = (start, stop) else {
        panic!("static/index.html has no modulepreload block");
    };
    let expected = expected_block(&graph);
    if html[start..stop] != expected && std::env::var_os("MAIDAN_WRITE_SNAPSHOTS").is_some() {
        let rewritten = format!("{}{}{}", &html[..start], expected, &html[stop..]);
        std::fs::write(&path, rewritten).unwrap();
        return;
    }
    assert_eq!(
        html[start..stop],
        expected,
        "modulepreload drift in static/index.html — rerun with MAIDAN_WRITE_SNAPSHOTS=1"
    );
}

#[test]
fn every_preloaded_module_is_one_the_server_serves() {
    let app =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs")).unwrap();
    for name in module_graph() {
        assert!(
            app.contains(&format!("include_str!(\"../static/ui/{name}\")")),
            "{name} is preloaded but /ui/static/{name} is not served"
        );
    }
}

#[test]
fn the_import_scanner_reads_static_and_dynamic_imports() {
    let found = imports("import { a } from \"./a.js\";\nconst b = await import('./b.js');\nimport \"./c.js\";\nimport x from \"https://x/y.js\";");
    assert_eq!(
        found.into_iter().collect::<BTreeSet<_>>(),
        ["a.js", "b.js", "c.js"].map(String::from).into()
    );
}
