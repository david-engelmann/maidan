//! Every `*_audited` store method checks its audit row's scope.
//!
//! The guard is `audit::append_scoped` on each backend, which resolves what
//! the write changed to a workspace and refuses a row scoped elsewhere. This
//! reads the source, so a new audited method, or an old one rewritten, fails
//! here when it writes its audit row any other way: by `INSERT INTO
//! maidan_audit` outside `audit.rs`, through the unguarded `append_on` or
//! `append_counted`, or through no guarded write at all.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const BACKENDS: [&str; 2] = ["sqlite", "postgres"];

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Each backend module's source, by module name.
fn modules(backend: &str) -> BTreeMap<String, String> {
    let dir = src().join(backend);
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(&dir).expect("backend dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            out.insert(name, read(&path));
        }
    }
    out
}

/// The body of `fn name` in `source`: from its signature to the next
/// top-level item.
fn body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let at = [
        format!("pub async fn {name}("),
        format!("pub(crate) async fn {name}("),
        format!("async fn {name}("),
    ]
    .iter()
    .find_map(|sig| source.find(sig.as_str()))?;
    let rest = &source[at..];
    let end = rest[1..]
        .find("\npub async fn ")
        .into_iter()
        .chain(rest[1..].find("\npub(crate) async fn "))
        .chain(rest[1..].find("\nasync fn "))
        .chain(rest[1..].find("\nfn "))
        .chain(rest[1..].find("\npub fn "))
        .min()
        .map_or(rest.len(), |i| i + 1);
    Some(&rest[..end])
}

/// Whether `fn name` in `module` writes through the guard, directly, through
/// the `audited!` macro (which does), or through a function it calls in the
/// same module.
fn guarded(mods: &BTreeMap<String, String>, module: &str, name: &str, depth: u8) -> bool {
    let Some(source) = mods.get(module) else {
        return false;
    };
    let Some(text) = body(source, name) else {
        return false;
    };
    if text.contains("append_scoped(") || text.contains("audited!(") {
        return true;
    }
    depth > 0
        && text
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| *word != name && !word.is_empty())
            .any(|callee| guarded(mods, module, callee, depth - 1))
}

/// Each `*_audited` method in the delegation list, and the backend function
/// (`module::function`) it calls.
fn audited_methods() -> Vec<(String, String, String)> {
    let delegate = read(&src().join("delegate.rs"));
    let mut out = Vec::new();
    for (at, _) in delegate.match_indices("async fn ") {
        let rest = &delegate[at + "async fn ".len()..];
        let method = &rest[..rest.find('(').expect("signature")];
        if !method.ends_with("_audited") {
            continue;
        }
        let open = rest.find('{').expect("body");
        let close = rest[open..].find("\n            }").expect("end") + open;
        let call = rest[open + 1..close].trim();
        let path = call
            .split('(')
            .next()
            .expect("call")
            .trim()
            .trim_start_matches("crate::")
            .trim_start_matches("self::");
        let mut parts = path.rsplit("::");
        let function = parts.next().expect("function").to_string();
        let module = parts.next().unwrap_or_default().to_string();
        out.push((method.to_string(), module, function));
    }
    out
}

#[test]
fn every_audited_method_writes_through_the_scope_guard() {
    let methods = audited_methods();
    let trait_methods = read(&src().join("store.rs")).matches("_audited(").count();
    assert!(
        methods.len() >= 50,
        "found {} audited methods",
        methods.len()
    );
    assert!(trait_methods >= methods.len());
    for backend in BACKENDS {
        let mods = modules(backend);
        let skipped: Vec<_> = methods
            .iter()
            .filter(|(_, module, function)| !guarded(&mods, module, function, 3))
            .map(|(method, module, function)| format!("{method} -> {module}::{function}"))
            .collect();
        assert!(
            skipped.is_empty(),
            "{backend}: audited methods that skip the scope guard: {skipped:#?}"
        );
    }
}

#[test]
fn only_the_guard_reaches_the_audit_table() {
    for backend in BACKENDS {
        for (module, source) in modules(backend) {
            if module == "audit" {
                for helper in ["async fn append_on(", "async fn append_counted("] {
                    let at = source.find(helper).expect("helper");
                    let line = source[..at].rsplit('\n').next().unwrap_or_default();
                    assert!(
                        !line.contains("pub"),
                        "{backend}: {helper} must stay private to audit.rs"
                    );
                }
                continue;
            }
            assert!(
                !source.contains("INSERT INTO maidan_audit"),
                "{backend}/{module}.rs writes the audit table without the scope guard"
            );
            for unguarded in ["audit::append_on(", "audit::append_counted("] {
                assert!(
                    !source.contains(unguarded),
                    "{backend}/{module}.rs calls {unguarded} around the scope guard"
                );
            }
        }
    }
}
