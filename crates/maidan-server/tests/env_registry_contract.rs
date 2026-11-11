//! `env_registry` is the list boot checks `MAIDAN_*` variables against (F-52),
//! so it has to be complete and nothing more. Every name the server's crates
//! read must be on it, or boot refuses a variable the server uses. Every name a
//! deploy file, script, SDK or live doc mentions must be on it, or that file is
//! either wrong or starts a server that refuses to boot. A listed name nothing
//! mentions is dead and is removed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use maidan_env::{ALLOW_UNKNOWN_ENV, SECRET_FILE_ENV, SERVER_ENV, TOLERATED_ENV};

const REGISTRY: &str = "crates/maidan-env/src/lib.rs";

/// The crates linked into the server binary. `maidan-cli` is its own binary.
const SERVER_CRATES: &[&str] = &[
    "maidan-a2a",
    "maidan-artifacts",
    "maidan-auth",
    "maidan-bus",
    "maidan-env",
    "maidan-fsm",
    "maidan-mcp",
    "maidan-observability",
    "maidan-router",
    "maidan-search",
    "maidan-server",
    "maidan-store",
    "maidan-types",
    "maidan-wasi",
];

/// Everything that names a variable for a server, or for a shell that starts
/// one. History (retros, cluster plans, the changelog) is left out: it names
/// variables that no longer exist, correctly.
const SCANNED: &[&str] = &[
    "crates",
    "compose.yaml",
    "compose.dev.yaml",
    "compose.pitr.yaml",
    "compose.quickstart.yaml",
    "compose.quickstart.insecure.yaml",
    "helm",
    "k8s",
    "docker",
    "scripts",
    ".github",
    "sdk",
    "examples",
    "Makefile",
    "README.md",
    "AGENTS.md",
    "CLAUDE.md",
    "CONTRIBUTING.md",
    "docs/Production.md",
    "docs/Configuration.md",
    "docs/Hosted Operations.md",
    "docs/Deploy.md",
    "docs/Integration.md",
    "docs/Embeddings.md",
    "docs/OIDC.md",
    "docs/Operations.md",
    "docs/Providers.md",
    "docs/WASI-Handlers.md",
];

/// The operator's reference. Every variable the server reads has a row there.
const CONFIGURATION: &str = "docs/Configuration.md";

/// Variables the server reads without the `MAIDAN_` prefix. Boot cannot refuse
/// a misspelling of these, so the list is short and closed: a new unprefixed
/// read fails `the_server_reads_no_unlisted_unprefixed_variable` until it is
/// named here, with the reason it has no prefix.
const UNPREFIXED_SERVER_ENV: &[(&str, &str)] = &[
    ("ARTIFACT_BACKEND", "operator setting, documented"),
    ("ARTIFACT_LOCALFS_ROOT", "operator setting, documented"),
    (
        "DATABASE_URL",
        "operator setting, documented; the sqlx convention",
    ),
    ("FEDERATION_DECRYPT_KEYS", "operator setting, documented"),
    ("FEDERATION_DISABLED", "operator setting, documented"),
    ("FEDERATION_ENCRYPTION_KEY", "operator setting, documented"),
    (
        "FEDERATION_POLL_INTERVAL_SECS",
        "operator setting, documented; compose.yaml shortens it for the federation smoke",
    ),
    ("INDEXER_STALE_SECS", "operator setting, documented"),
    (
        "KUBERNETES_SERVICE_HOST",
        "injected by Kubernetes, not configuration",
    ),
    ("OTLP_ENDPOINT", "operator setting, documented"),
    ("OTLP_METRICS_ENDPOINT", "operator setting, documented"),
    ("OTLP_METRICS_INTERVAL_SECS", "operator setting, documented"),
    ("OTLP_SERVICE_NAME", "operator setting, documented"),
    ("S3_REGION", "operator setting, documented"),
    (
        "UPDATE_PROFILE_GOLDEN",
        "a test regenerating its golden, not configuration",
    ),
];

const SKIPPED_DIRS: &[&str] = &["target", "node_modules", ".venv", "__pycache__", "dist"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                files(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

/// Whole variable names in `text`. A prefix written as a pattern
/// (`MAIDAN_SMTP_*`) is not a name and is skipped.
fn names_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find("MAIDAN_") {
        let start = from + offset;
        let preceded = start > 0 && {
            let c = bytes[start - 1];
            c.is_ascii_alphanumeric() || c == b'_'
        };
        let mut end = start + "MAIDAN_".len();
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        let pattern = bytes.get(end) == Some(&b'*') || bytes.get(end) == Some(&b'{');
        let name = text[start..end].trim_end_matches('_');
        if !preceded && !pattern && name.len() > "MAIDAN_".len() {
            found.push(name.to_string());
        }
        from = end;
    }
    found
}

fn mentions() -> BTreeMap<String, BTreeSet<String>> {
    let root = repo_root();
    let mut all = Vec::new();
    for entry in SCANNED {
        files(&root.join(entry), &mut all);
    }
    let mut mentions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in all {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative == REGISTRY {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for name in names_in(&text) {
            mentions.entry(name).or_default().insert(relative.clone());
        }
    }
    mentions
}

#[test]
fn every_variable_the_repo_mentions_is_registered() {
    let unregistered: Vec<String> = mentions()
        .into_iter()
        .filter(|(name, _)| !SERVER_ENV.contains(&name.as_str()))
        .filter(|(name, _)| !TOLERATED_ENV.contains(&name.as_str()))
        .map(|(name, files)| format!("{name} ({})", Vec::from_iter(files).join(", ")))
        .collect();
    assert!(
        unregistered.is_empty(),
        "boot would refuse these; add each to SERVER_ENV (the server reads it) or \
         TOLERATED_ENV (something else does), or fix the name:\n  {}",
        unregistered.join("\n  ")
    );
}

#[test]
fn every_server_variable_is_read_by_a_server_crate() {
    let root = repo_root();
    let mut sources = Vec::new();
    for krate in SERVER_CRATES {
        files(&root.join("crates").join(krate).join("src"), &mut sources);
    }
    let mut read = BTreeSet::new();
    for path in sources {
        if path.ends_with("env_registry.rs")
            || path.ends_with("maidan-env/src/lib.rs")
            || path.extension().is_none_or(|e| e != "rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for name in SERVER_ENV {
            if text.contains(&format!("\"{name}\"")) {
                read.insert(*name);
            }
        }
    }
    // The escape hatch is read through its constant, by the boot check itself.
    read.insert(ALLOW_UNKNOWN_ENV);
    let unread: Vec<&str> = SERVER_ENV
        .iter()
        .copied()
        .filter(|name| !read.contains(name))
        .collect();
    assert!(
        unread.is_empty(),
        "on SERVER_ENV but read by no server crate; move it to TOLERATED_ENV or \
         remove it: {unread:?}"
    );
}

#[test]
fn every_tolerated_variable_is_mentioned_somewhere() {
    let mentions = mentions();
    let dead: Vec<&str> = TOLERATED_ENV
        .iter()
        .copied()
        .filter(|name| !mentions.contains_key(*name))
        .collect();
    assert!(
        dead.is_empty(),
        "on TOLERATED_ENV but nothing mentions it; remove it: {dead:?}"
    );
}

#[test]
fn names_in_skips_patterns_and_embedded_matches() {
    assert_eq!(
        names_in("set MAIDAN_BIND, not MAIDAN_SMTP_* or XMAIDAN_A or MAIDAN_{KEY}"),
        vec!["MAIDAN_BIND".to_string()]
    );
}

/// The server crates' library sources, without the registry itself.
fn server_sources() -> Vec<(PathBuf, String)> {
    let root = repo_root();
    let mut paths = Vec::new();
    for krate in SERVER_CRATES {
        files(&root.join("crates").join(krate).join("src"), &mut paths);
    }
    paths
        .into_iter()
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .filter(|path| !path.ends_with("maidan-env/src/lib.rs"))
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            (path, text)
        })
        .collect()
}

/// The names `text` reads with `env::var("…")` or `env::var_os("…")`.
fn env_reads_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for call in ["var(\"", "var_os(\""] {
        let mut from = 0;
        while let Some(offset) = text[from..].find(call) {
            let start = from + offset + call.len();
            let len = text[start..]
                .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(text.len() - start);
            if len > 0 && text[start + len..].starts_with('"') {
                found.push(text[start..start + len].to_string());
            }
            from = start;
        }
    }
    found
}

/// The variable names `doc` gives a row or a mention to, written as code.
fn documented_in(doc: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for chunk in doc.split('`').skip(1).step_by(2) {
        let name = chunk.split('=').next().unwrap_or("");
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            names.insert(name.to_string());
        }
    }
    names
}

fn configuration_names() -> BTreeSet<String> {
    let doc = std::fs::read_to_string(repo_root().join(CONFIGURATION)).unwrap();
    documented_in(&doc)
}

/// DOCS-003: the reference is the operator's only list, so a variable the
/// server reads and the page leaves out is one nobody can find. A secret's
/// `_FILE` form counts as documented with its secret, since the page states
/// the rule once for all of them.
#[test]
fn every_server_variable_is_documented_in_the_configuration_reference() {
    let documented = configuration_names();
    let file_forms: BTreeMap<&str, &str> = SECRET_FILE_ENV
        .iter()
        .map(|(name, file)| (*file, *name))
        .collect();
    let undocumented: Vec<&str> = SERVER_ENV
        .iter()
        .copied()
        .filter(|name| {
            let name = file_forms.get(name).copied().unwrap_or(name);
            !documented.contains(name)
        })
        .collect();
    assert!(
        undocumented.is_empty(),
        "the server reads these and {CONFIGURATION} has no row for them; \
         document each one or stop reading it: {undocumented:?}"
    );
}

/// OPS-006: boot refuses an unknown `MAIDAN_*` name, and nothing else. An
/// unprefixed read is invisible to that check, so it has to be on the short
/// list above, and an operator setting on that list has to be documented.
#[test]
fn the_server_reads_no_unlisted_unprefixed_variable() {
    let listed: BTreeSet<&str> = UNPREFIXED_SERVER_ENV
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let mut read: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let root = repo_root();
    for (path, text) in server_sources() {
        for name in env_reads_in(&text) {
            if name.starts_with("MAIDAN_") {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            read.entry(name).or_default().insert(relative);
        }
    }
    let unlisted: Vec<String> = read
        .iter()
        .filter(|(name, _)| !listed.contains(name.as_str()))
        .map(|(name, files)| format!("{name} ({})", Vec::from_iter(files.clone()).join(", ")))
        .collect();
    assert!(
        unlisted.is_empty(),
        "the server reads these without the MAIDAN_ prefix, so boot cannot refuse a \
         misspelling; name them MAIDAN_* and register them, or add each to \
         UNPREFIXED_SERVER_ENV with the reason:\n  {}",
        unlisted.join("\n  ")
    );
    let unread: Vec<&str> = listed
        .iter()
        .copied()
        .filter(|name| !read.contains_key(*name))
        .collect();
    assert!(
        unread.is_empty(),
        "on UNPREFIXED_SERVER_ENV but read nowhere: {unread:?}"
    );
    let documented = configuration_names();
    let undocumented: Vec<&str> = UNPREFIXED_SERVER_ENV
        .iter()
        .filter(|(_, reason)| reason.starts_with("operator setting"))
        .map(|(name, _)| *name)
        .filter(|name| !documented.contains(*name))
        .collect();
    assert!(
        undocumented.is_empty(),
        "{CONFIGURATION} has no row for these unprefixed settings: {undocumented:?}"
    );
}

#[test]
fn env_reads_in_finds_literal_reads_only() {
    assert_eq!(
        env_reads_in(
            r#"std::env::var("DATABASE_URL"); env::var_os("X_Y"); var(name); var("lower")"#
        ),
        vec!["DATABASE_URL".to_string(), "X_Y".to_string()]
    );
}

#[test]
fn documented_in_reads_code_spans_and_assignments() {
    let names =
        documented_in("set `MAIDAN_BIND`, `MAIDAN_OUTBOX_RELAY=0`, not `json` or `MAIDAN_SMTP_*`");
    assert_eq!(
        names,
        BTreeSet::from(["MAIDAN_BIND".to_string(), "MAIDAN_OUTBOX_RELAY".to_string()])
    );
}
