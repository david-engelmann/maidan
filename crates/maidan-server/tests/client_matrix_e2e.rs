//! The MCP client matrix as test config. `contracts/mcp-clients.json` says how
//! each client this project is tested against authenticates. This test checks
//! the file's shape, checks that `docs/Clients.md` gives every client a recipe,
//! a matrix row and (where flagged) a step in the release check, and then
//! connects to a real server with auth enabled the way each client does.
//! `gemini-extension.json` at the repository root is checked here too: it
//! parses, names no host of its own, reads the instance and the token from the
//! environment, and reaches a real server once those are filled in.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{dev_anonymous, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn matrix() -> Value {
    let text = std::fs::read_to_string(repo().join("contracts/mcp-clients.json"))
        .expect("contracts/mcp-clients.json");
    serde_json::from_str(&text).expect("mcp-clients.json is JSON")
}

fn clients(matrix: &Value) -> Vec<Value> {
    matrix["clients"].as_array().expect("clients").clone()
}

/// The text of `docs/Clients.md` under `heading`, up to the next heading of
/// the same or a higher level.
fn section<'a>(doc: &'a str, heading: &str) -> Option<&'a str> {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let start = doc.find(&format!("\n{heading}\n"))? + heading.len() + 2;
    let rest = &doc[start..];
    let end = rest
        .match_indices("\n#")
        .find(|(at, _)| rest[at + 1..].chars().take_while(|c| *c == '#').count() <= level)
        .map_or(rest.len(), |(at, _)| at);
    Some(&rest[..end])
}

#[test]
fn every_client_row_is_well_formed() {
    let matrix = matrix();
    let modes: HashSet<&str> = matrix["auth_modes"]
        .as_object()
        .expect("auth_modes")
        .keys()
        .map(String::as_str)
        .collect();
    let elicitation: HashSet<&str> = matrix["elicitation_values"]
        .as_array()
        .expect("elicitation_values")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut ids = HashSet::new();
    let mut found = Vec::new();
    for client in clients(&matrix) {
        let id = client["id"].as_str().unwrap_or_default().to_string();
        if !ids.insert(id.clone()) {
            found.push(format!("{id}: listed twice"));
        }
        for field in ["name", "note", "checked"] {
            if client[field].as_str().is_none_or(|v| v.trim().is_empty()) {
                found.push(format!("{id}: no {field}"));
            }
        }
        if !client["auth"].as_str().is_some_and(|m| modes.contains(m)) {
            found.push(format!("{id}: auth is not one of {modes:?}"));
        }
        if !client["elicitation"]
            .as_str()
            .is_some_and(|e| elicitation.contains(e))
        {
            found.push(format!("{id}: elicitation is not one of {elicitation:?}"));
        }
        if !matches!(
            client["endpoint"].as_str(),
            Some("/mcp" | "/mcp/streamable")
        ) {
            found.push(format!("{id}: endpoint is not an MCP endpoint"));
        }
        if !client["source"]
            .as_str()
            .is_some_and(|s| s.starts_with("https://"))
        {
            found.push(format!("{id}: source is not the client's documentation"));
        }
        if !client["release_check"].is_boolean() {
            found.push(format!("{id}: release_check is not a boolean"));
        }
        if client["checked"]
            .as_str()
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .is_none()
        {
            found.push(format!("{id}: checked is not a date"));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn every_client_has_a_recipe_a_matrix_row_and_its_release_step() {
    let doc = std::fs::read_to_string(repo().join("docs/Clients.md")).expect("docs/Clients.md");
    let release = section(&doc, "## The release check").expect("a release check section");
    let mut found = Vec::new();
    for client in clients(&matrix()) {
        let name = client["name"].as_str().unwrap_or_default();
        if !doc.contains(&format!("\n| {name} |")) {
            found.push(format!("{name}: no row in the client matrix"));
        }
        match section(&doc, &format!("### {name}")) {
            None => found.push(format!("{name}: no recipe")),
            Some(recipe) => {
                let recipe = recipe.split_whitespace().collect::<Vec<_>>().join(" ");
                match client["auth"].as_str() {
                    Some("bearer-header")
                        if !(recipe.contains("Authorization") && recipe.contains("Bearer")) =>
                    {
                        found.push(format!("{name}: its recipe sends no bearer header"));
                    }
                    Some("anonymous-dev") if !recipe.contains("dev instance") => {
                        found.push(format!("{name}: its recipe does not use a dev instance"));
                    }
                    _ => {}
                }
            }
        }
        if client["release_check"] == json!(true) && !release.contains(name) {
            found.push(format!(
                "{name}: flagged for the release check, but not in it"
            ));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

fn gemini_extension() -> Value {
    let text = std::fs::read_to_string(repo().join("gemini-extension.json"))
        .expect("gemini-extension.json at the repository root");
    serde_json::from_str(&text).expect("gemini-extension.json is JSON")
}

/// The one URL and header the extension sends, as Gemini CLI fills them in
/// from the environment.
const GEMINI_URL_PREFIX: &str = "${MAIDAN_URL}";
const GEMINI_AUTHORIZATION: &str = "Bearer ${MAIDAN_TOKEN}";

#[test]
fn the_gemini_extension_names_the_users_own_instance_and_an_environment_token() {
    let manifest = gemini_extension();
    let gemini = clients(&matrix())
        .into_iter()
        .find(|c| c["id"] == "gemini-cli")
        .expect("a gemini-cli row in the matrix");
    let endpoint = gemini["endpoint"].as_str().unwrap_or_default();
    let mut found = Vec::new();

    // Gemini CLI's extension reference: lowercase letters, numbers and dashes,
    // matching the directory the extension is installed under
    // (~/.gemini/extensions/<name>), which is the repository's name.
    let name = manifest["name"].as_str().unwrap_or_default();
    if name != "maidan"
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        found.push(format!(
            "name {name:?} is not the repository's name, `maidan`"
        ));
    }
    if manifest["version"]
        .as_str()
        .is_none_or(|v| v.trim().is_empty())
    {
        found.push("no version, which Gemini CLI refuses to load".into());
    }
    if let Some(context) = manifest.get("contextFileName") {
        match context.as_str() {
            Some(file)
                if !file.contains("..")
                    && !file.starts_with('/')
                    && repo().join(file).is_file() => {}
            _ => found.push(format!(
                "contextFileName {context} is not a file in the repository"
            )),
        }
    }

    let servers = manifest["mcpServers"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if servers.len() != 1 || !servers.contains_key("maidan") {
        found.push("mcpServers is not the one `maidan` server".into());
    }
    for (key, server) in &servers {
        let fields: HashSet<&str> = server
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        if fields != HashSet::from(["httpUrl", "headers"]) {
            found.push(format!(
                "{key}: has {fields:?}, not just httpUrl and headers (no url, which the \
                 documentation calls SSE, no command, and no trust)"
            ));
        }
        let url = server["httpUrl"].as_str().unwrap_or_default();
        if url != format!("{GEMINI_URL_PREFIX}{endpoint}") {
            found.push(format!(
                "{key}: httpUrl is {url:?}, not the user's instance at {GEMINI_URL_PREFIX}{endpoint}"
            ));
        }
        let headers = server["headers"].as_object().cloned().unwrap_or_default();
        if headers.len() != 1 || headers.get("Authorization") != Some(&json!(GEMINI_AUTHORIZATION))
        {
            found.push(format!(
                "{key}: headers are {headers:?}, not one Authorization header read from MAIDAN_TOKEN"
            ));
        }
    }
    // A fixed endpoint anywhere in the file would send someone's token to a
    // host they did not choose.
    let text = manifest.to_string();
    if text.contains("://") || text.contains("localhost") || text.contains("127.0.0.1") {
        found.push("the manifest names a host".into());
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_gemini_cli_recipe_documents_the_extension_install_and_its_fallback() {
    let doc = std::fs::read_to_string(repo().join("docs/Clients.md")).expect("docs/Clients.md");
    let recipe = section(&doc, "### Gemini CLI").expect("a Gemini CLI recipe");
    let recipe = recipe.split_whitespace().collect::<Vec<_>>().join(" ");
    for needle in [
        "gemini extensions install https://github.com/david-engelmann/maidan",
        "gemini extensions list",
        "gemini mcp list",
        "MAIDAN_URL",
        "MAIDAN_TOKEN",
        "gemini-extension.json",
        // The manifest cannot refuse plain http, so the recipe says to use https.
        "`https://` address for any instance that is not on your own machine",
    ] {
        assert!(
            recipe.contains(needle),
            "the Gemini CLI recipe does not mention {needle}"
        );
    }
}

struct Server {
    base: String,
    client: reqwest::Client,
    synthetic: maidan_types::WorkspaceId,
    bearer: String,
}

async fn spawn() -> Server {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let synthetic = store
        .create_workspace(NewWorkspace {
            name: "synthetic-clients".into(),
        })
        .await
        .unwrap();
    store
        .create_channel(NewChannel {
            workspace_id: synthetic.id,
            name: "fixtures".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let team = store
        .create_workspace(NewWorkspace {
            name: "team".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: team.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: team.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();

    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.dev_anonymous_reader = Some(
        dev_anonymous::reader_for(store.as_ref(), synthetic.id)
            .await
            .unwrap(),
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        synthetic: synthetic.id,
        bearer: secret.as_str().to_string(),
    }
}

impl Server {
    async fn rpc(
        &self,
        endpoint: &str,
        bearer: Option<&str>,
        method: &str,
        params: Value,
    ) -> Value {
        let mut request = self
            .client
            .post(format!("{}{endpoint}", self.base))
            .header("Accept", "application/json")
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{method} on {endpoint}");
        response.json().await.unwrap()
    }
}

#[tokio::test]
async fn the_gemini_extension_reaches_a_real_server_once_the_environment_fills_it_in() {
    let server = spawn().await;
    let manifest = gemini_extension();
    let entry = &manifest["mcpServers"]["maidan"];
    let endpoint = entry["httpUrl"]
        .as_str()
        .and_then(|u| u.strip_prefix(GEMINI_URL_PREFIX))
        .expect("httpUrl starts with the instance URL");
    let bearer = entry["headers"]["Authorization"]
        .as_str()
        .map(|h| h.replace("${MAIDAN_TOKEN}", &server.bearer))
        .and_then(|h| h.strip_prefix("Bearer ").map(str::to_string))
        .expect("a bearer header");
    let listed = server
        .rpc(endpoint, Some(&bearer), "tools/list", json!({}))
        .await;
    assert!(
        listed["result"]["tools"]
            .as_array()
            .is_some_and(|t| t.iter().any(|t| t["name"] == "whoami")),
        "the extension's server lists Maidan's tools: {listed}"
    );
}

#[tokio::test]
async fn every_client_connects_the_way_the_matrix_says() {
    let server = spawn().await;
    for client in clients(&matrix()) {
        let name = client["name"].as_str().unwrap_or_default();
        let endpoint = client["endpoint"].as_str().unwrap_or_default();
        match client["auth"].as_str() {
            Some("bearer-header") => {
                let bearer = Some(server.bearer.as_str());
                let listed = server.rpc(endpoint, bearer, "tools/list", json!({})).await;
                let tools = listed["result"]["tools"].as_array().expect("tools");
                assert!(
                    tools.iter().any(|t| !maidan_mcp::tools::is_read_only(
                        t["name"].as_str().unwrap_or_default()
                    )),
                    "{name}: a token-holding client sees the tools that write"
                );
                let me = server
                    .rpc(
                        endpoint,
                        bearer,
                        "tools/call",
                        json!({ "name": "whoami", "arguments": {} }),
                    )
                    .await;
                assert!(
                    me["result"]["content"][0]["text"]
                        .as_str()
                        .is_some_and(|t| t.contains("member_id")),
                    "{name}: whoami answers with the token's member: {me}"
                );
            }
            Some("anonymous-dev") => {
                let listed = server.rpc(endpoint, None, "tools/list", json!({})).await;
                let tools = listed["result"]["tools"].as_array().expect("tools");
                assert!(
                    !tools.is_empty(),
                    "{name}: discovers tools with no credential"
                );
                for tool in tools {
                    assert_eq!(
                        tool["securitySchemes"],
                        json!([{ "type": "noauth" }]),
                        "{name}: {}",
                        tool["name"]
                    );
                }
                let read = server
                    .rpc(
                        endpoint,
                        None,
                        "tools/call",
                        json!({ "name": "list_channels", "arguments": { "workspace_id": server.synthetic } }),
                    )
                    .await;
                assert!(
                    read.to_string().contains("fixtures"),
                    "{name}: reads the synthetic workspace: {read}"
                );
            }
            other => panic!("{name}: unknown auth mode {other:?}"),
        }
    }
}
