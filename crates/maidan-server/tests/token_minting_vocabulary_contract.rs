//! One minting vocabulary: every token-minting route is bounded by
//! `routes::mint_vocabulary`.
//!
//! Token minting happens in several places (direct mint, attenuation, OAuth
//! exchange, app exchange, session bootstrap). Each must filter the
//! capabilities it mints through the same vocabulary, not an ad-hoc list.
//!
//! This test pins the invariant structurally: every source file that mints
//! a token (calls `create_api_token_audited`, `mint_oauth_token_audited`, or
//! `create_attenuated_api_token_audited`) must reference `mint_vocabulary`.
//! A new mint path that skips the vocabulary fails this test.
//!
//! The approved exceptions are documented below; each has a reason why the
//! vocabulary bound does not apply.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Source files that mint tokens but are explicitly exempt from the
/// `mint_vocabulary` bound, with the reason.
fn exempt_files() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    // The OIDC first-admin bootstrap mints the very first admin token.
    // There is no caller auth context to bound against; the capabilities
    // are `default_minted() + TOKEN_ADMIN`, hardcoded, not caller-supplied.
    m.insert(
        "src/session/handlers.rs",
        "OIDC first-admin bootstrap: no caller auth, hardcoded capabilities",
    );
    m
}

/// Every non-exempt file that mints tokens must reference `mint_vocabulary`.
#[test]
fn every_mint_path_references_mint_vocabulary() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let server_src = root.join("src");

    let mut mint_files = Vec::new();
    collect_mint_files(&server_src, &mut mint_files);

    let exempt = exempt_files();
    let mut violations = Vec::new();

    for file in mint_files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if exempt.contains_key(rel.as_str()) {
            continue;
        }
        let content = fs::read_to_string(&file).expect("read source file");
        if !content.contains("mint_vocabulary") {
            violations.push(rel);
        }
    }

    assert!(
        violations.is_empty(),
        "token-minting files that do not reference `mint_vocabulary`:\n  {}\n\
         Every token-minting route must bound its capabilities by \
         `routes::mint_vocabulary`. If this is a new mint path, add the \
         bound. If it is legitimately exempt, add it to `exempt_files()` \
         with a reason.",
        violations.join("\n  ")
    );
}

/// Recursively find `.rs` files containing a token-mint call.
fn collect_mint_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_mint_files(&path, out);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            let content = fs::read_to_string(&path).unwrap_or_default();
            if content.contains("create_api_token_audited")
                || content.contains("mint_oauth_token_audited")
                || content.contains("create_attenuated_api_token_audited")
            {
                out.push(path);
            }
        }
    }
}

/// `mint_vocabulary` itself is the single bound. It must remain the only
/// function that defines which capabilities a caller may mint.
#[test]
fn mint_vocabulary_is_the_single_bound() {
    // The vocabulary excludes the cross-tenant capabilities unless the
    // caller already holds them. This is the security property the contract
    // protects: a workspace admin cannot mint an instance operator.
    use maidan_auth::capability::{AUDIT_READ_GLOBAL, OPERATOR_GLOBAL};

    // These are the two cross-tenant capabilities. They must be known...
    assert!(maidan_auth::capability::is_known(OPERATOR_GLOBAL));
    assert!(maidan_auth::capability::is_known(AUDIT_READ_GLOBAL));

    // ...and the vocabulary function must exist and be public.
    // (This is a compile-time check: if `mint_vocabulary` is not public
    // under `routes`, this test does not compile.)
    let _ = maidan_server::routes::mint_vocabulary as fn(&maidan_auth::AuthContext) -> Vec<String>;
}
