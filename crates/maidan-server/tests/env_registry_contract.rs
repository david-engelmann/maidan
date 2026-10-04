//! `env_registry` is the list boot checks `MAIDAN_*` variables against (F-52),
//! so it has to be complete and nothing more. Every name the server's crates
//! read must be on it, or boot refuses a variable the server uses. Every name a
//! deploy file, script, SDK or live doc mentions must be on it, or that file is
//! either wrong or starts a server that refuses to boot. A listed name nothing
//! mentions is dead and is removed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use maidan_env::{ALLOW_UNKNOWN_ENV, SERVER_ENV, TOLERATED_ENV};

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
    "docs/Deploy.md",
    "docs/Integration.md",
    "docs/Embeddings.md",
    "docs/OIDC.md",
    "docs/Operations.md",
    "docs/Providers.md",
    "docs/WASI-Handlers.md",
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
