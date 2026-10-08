//! Keeps the Keycloak sign-in recipe (docs/OIDC.md, "Recipe: Keycloak") honest.
//!
//! The recipe was run end to end against a live Keycloak; this test pins the
//! parts that can drift without anyone rerunning it: the realm file's client
//! settings, the routes the docs and the smoke script call, the env names the
//! recipe sets, and the token request body it shows.

use std::{collections::BTreeSet, fs, path::PathBuf};

use maidan_env::SERVER_ENV;
use maidan_server::dto::MintApiToken;
use serde_json::Value;

const REALM_FILE: &str = "examples/keycloak/maidan-realm.json";
const SMOKE_SCRIPT: &str = "scripts/keycloak-oidc-smoke.sh";
const CALLBACK: &str = "/auth/oidc/callback";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn realm() -> Value {
    serde_json::from_str(&read(REALM_FILE)).expect("realm file is JSON")
}

/// The "Recipe: Keycloak" section of docs/OIDC.md, up to the next `## `.
fn recipe() -> String {
    let doc = read("docs/OIDC.md");
    let start = doc
        .find("## Recipe: Keycloak\n")
        .expect("docs/OIDC.md has a \"Recipe: Keycloak\" section");
    let rest = &doc[start + 3..];
    let end = rest.find("\n## ").map_or(rest.len(), |i| i + 1);
    rest[..end].to_string()
}

/// `KEY=value` lines of the recipe's Maidan env block, the fenced block that
/// sets `MAIDAN_OIDC_ENABLED` (values up to whitespace).
fn recipe_env() -> Vec<(String, String)> {
    let recipe = recipe();
    let block = recipe
        .split("```")
        .find(|b| b.contains("\nMAIDAN_OIDC_ENABLED="))
        .expect("recipe has a Maidan env block")
        .to_string();
    block
        .lines()
        .filter(|l| l.starts_with("MAIDAN_"))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.to_string(), v.split_whitespace().next()?.to_string()))
        })
        .collect()
}

fn env_value(name: &str) -> String {
    recipe_env()
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("recipe env block sets {name}"))
}

/// The single client in the realm, which must be the one the recipe names.
fn client() -> Value {
    let realm = realm();
    let clients = realm["clients"].as_array().expect("realm has clients");
    assert_eq!(
        clients.len(),
        1,
        "the realm file defines exactly one client"
    );
    clients[0].clone()
}

/// Route templates mounted in app.rs, with every `{param}` reduced to `{}`.
fn mounted_routes() -> BTreeSet<String> {
    let app = read("crates/maidan-server/src/app.rs");
    app.split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| s.starts_with('/') && !s.contains(' '))
        .map(normalize_template)
        .collect()
}

fn normalize_template(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            if (seg.starts_with('{') && seg.ends_with('}')) || seg.starts_with('$') {
                "{}"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Maidan paths a shell text calls as `"$<base>/path..."`, query stripped.
fn called_paths(text: &str, base: &str) -> BTreeSet<String> {
    let needle = format!("${base}/");
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(i) = rest.find(&needle) {
        let after = &rest[i + needle.len() - 1..];
        let end = after
            .find(|c: char| c == '?' || c == '"' || c.is_whitespace())
            .unwrap_or(after.len());
        out.insert(normalize_template(&after[..end]));
        rest = &after[end..];
    }
    out
}

#[test]
fn the_realm_client_is_public_code_flow_with_s256_pkce() {
    let realm = realm();
    assert_eq!(realm["realm"], "maidan");
    assert_eq!(realm["enabled"], true);
    assert_eq!(
        realm["registrationAllowed"], false,
        "self-registration off: only people an admin adds can sign in"
    );

    let c = client();
    assert_eq!(c["clientId"], "maidan");
    assert_eq!(c["protocol"], "openid-connect");
    assert_eq!(
        c["publicClient"], true,
        "Maidan sends no client secret unless MAIDAN_OIDC_CLIENT_SECRET is set"
    );
    assert_eq!(c["standardFlowEnabled"], true, "authorization code flow on");
    for off in [
        "implicitFlowEnabled",
        "directAccessGrantsEnabled",
        "serviceAccountsEnabled",
    ] {
        assert_eq!(c[off], false, "{off} must stay off");
    }
    assert_eq!(c["attributes"]["pkce.code.challenge.method"], "S256");
    let scopes: Vec<&str> = c["defaultClientScopes"]
        .as_array()
        .expect("default scopes")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for needed in ["profile", "email"] {
        assert!(
            scopes.contains(&needed),
            "Maidan asks for `openid email profile`; the client must grant {needed}"
        );
    }
}

#[test]
fn the_realm_file_carries_no_users_or_secrets() {
    let text = read(REALM_FILE);
    let realm = realm();
    assert!(
        realm.get("users").is_none(),
        "users are created per instance"
    );
    for word in ["\"secret\"", "\"credentials\"", "\"password\""] {
        assert!(!text.contains(word), "realm file must not carry {word}");
    }
}

#[test]
fn redirect_uris_are_exact_and_match_the_recipe_and_the_mounted_callback() {
    let c = client();
    let uris: Vec<&str> = c["redirectUris"]
        .as_array()
        .expect("redirect URIs")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(!uris.is_empty());
    for uri in &uris {
        assert!(!uri.contains('*'), "no wildcard redirect URI: {uri}");
        assert!(uri.ends_with(CALLBACK), "{uri} is not Maidan's callback");
    }
    assert!(
        mounted_routes().contains(CALLBACK),
        "app.rs no longer mounts {CALLBACK}"
    );
    assert!(
        uris.contains(&env_value("MAIDAN_OIDC_REDIRECT_URI").as_str()),
        "the recipe's MAIDAN_OIDC_REDIRECT_URI is not registered on the client"
    );
    assert_eq!(
        c["attributes"]["post.logout.redirect.uris"],
        env_value("MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI").as_str(),
        "post-logout redirect differs between the realm and the recipe"
    );
}

#[test]
fn the_recipe_env_is_known_to_the_server_and_matches_the_realm() {
    let env = recipe_env();
    assert!(env.len() >= 5, "recipe env block not found: {env:?}");
    for (name, _) in &env {
        assert!(
            SERVER_ENV.contains(&name.as_str()),
            "{name} is not a server env name; boot would refuse it"
        );
    }
    assert_eq!(env_value("MAIDAN_OIDC_ENABLED"), "1");
    assert_eq!(env_value("MAIDAN_OIDC_CLIENT_ID"), client()["clientId"]);
    let realm = realm();
    let realm_name = realm["realm"].as_str().expect("realm name");
    assert!(
        env_value("MAIDAN_OIDC_ISSUER").ends_with(&format!("/realms/{realm_name}")),
        "Keycloak's issuer is <base>/realms/<realm>"
    );
    assert_eq!(
        env_value("MAIDAN_OIDC_LINK_EMAIL"),
        "1",
        "the recipe links pre-provisioned members by verified email"
    );
    assert!(
        !env.iter().any(|(k, _)| k == "MAIDAN_OIDC_AUTO_PROVISION"),
        "the recipe keeps auto-provisioning off"
    );
}

#[test]
fn every_maidan_path_the_recipe_and_script_call_is_mounted() {
    let routes = mounted_routes();
    let script = read(SMOKE_SCRIPT);
    let script_paths = called_paths(&script, "BASE");
    let doc_paths = called_paths(&recipe(), "MAIDAN_URL");
    assert!(script_paths.len() >= 4, "script paths: {script_paths:?}");
    assert!(doc_paths.len() >= 2, "doc paths: {doc_paths:?}");
    let missing: Vec<&String> = script_paths
        .iter()
        .chain(&doc_paths)
        .filter(|p| !routes.contains(*p))
        .collect();
    assert!(missing.is_empty(), "not mounted in app.rs: {missing:?}");
}

#[test]
fn the_recipe_names_its_files_and_the_script_is_executable() {
    let recipe = recipe();
    assert!(recipe.contains(REALM_FILE), "recipe links {REALM_FILE}");
    assert!(recipe.contains(SMOKE_SCRIPT), "recipe names {SMOKE_SCRIPT}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(repo_root().join(SMOKE_SCRIPT))
            .expect("smoke script exists")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "{SMOKE_SCRIPT} is executable");
    }
}

#[test]
fn the_token_request_in_the_recipe_is_a_valid_scoped_expiring_mint() {
    let recipe = recipe();
    let start = recipe
        .find("-d '{\"label\"")
        .expect("recipe shows a token mint body")
        + 4;
    let end = start + recipe[start..].find('\'').expect("body closes");
    let body: Value = serde_json::from_str(&recipe[start..end]).expect("mint body is JSON");
    let keys: BTreeSet<&str> = body
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    let known: BTreeSet<&str> = [
        "label",
        "capabilities",
        "capability_set",
        "expires_at",
        "quotas",
    ]
    .into();
    assert!(keys.is_subset(&known), "unknown mint fields: {keys:?}");
    let mint: MintApiToken = serde_json::from_value(body).expect("deserializes as MintApiToken");
    assert!(mint.expires_at.is_some(), "the recipe's token expires");
    assert!(
        !mint.capabilities.is_empty() && mint.capability_set.is_none(),
        "the recipe's token is scoped to named capabilities"
    );
    for cap in &mint.capabilities {
        assert!(
            !cap.starts_with("token:"),
            "a person's client token must not mint tokens ({cap})"
        );
    }
}
