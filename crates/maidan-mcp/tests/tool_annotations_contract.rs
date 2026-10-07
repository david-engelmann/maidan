//! Every MCP tool declares a title and the four behaviour hints of the MCP tool
//! spec (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`),
//! and each declared value matches `tests/fixtures/tool-annotations.json`, the
//! reviewed table that gives every value a reason a reviewer can check against
//! the handler. Directories reject a listing whose hints do not match what its
//! tools do, and a client asks before a destructive call only if the hint says
//! so; a new tool cannot ship without a reviewed row. A tool's name fits
//! every client, its leading verb says whether it writes, and the few writes a
//! model could mistake for lookups are listed and say what they write.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::AuthContext;
use maidan_mcp::{JsonRpcRequest, McpServer, Profile};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const HINTS: [&str; 4] = [
    "readOnlyHint",
    "destructiveHint",
    "idempotentHint",
    "openWorldHint",
];

/// Claude and OpenAI clients refuse a tool name longer than this.
const NAME_MAX: usize = 64;

/// Claude Code calls a server's tool `mcp__<server>__<tool>`, so with the
/// server registered as `maidan` the prefix comes out of the same budget.
const CLAUDE_CODE_PREFIX: &str = "mcp__maidan__";

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tool-annotations.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&text).expect("tool-annotations.json")
}

fn reviewed() -> BTreeMap<String, Value> {
    fixture()["tools"]
        .as_object()
        .expect("tool-annotations.json has a tools object")
        .iter()
        .map(|(name, row)| (name.clone(), row.clone()))
        .collect()
}

fn catalog() -> BTreeMap<String, Value> {
    maidan_mcp::tools::catalog()
        .into_iter()
        .map(|tool| (tool["name"].as_str().expect("tool name").to_string(), tool))
        .collect()
}

/// What a tool's `annotations` must be, built from its reviewed row.
fn expected_annotations(row: &Value) -> Value {
    let mut expected = json!({ "title": row["title"] });
    for hint in HINTS {
        expected[hint] = row[hint]["value"].clone();
    }
    expected
}

/// Every problem with one served tool entry, so a failure names them all.
fn problems(name: &str, tool: &Value, row: Option<&Value>) -> Vec<String> {
    let mut found = Vec::new();
    let Some(annotations) = tool.get("annotations").and_then(Value::as_object) else {
        return vec![format!("{name} has no annotations")];
    };
    match annotations.get("title").and_then(Value::as_str) {
        Some(title) if !title.trim().is_empty() => {}
        _ => found.push(format!("{name} has no title")),
    }
    for hint in HINTS {
        if !annotations.get(hint).is_some_and(Value::is_boolean) {
            found.push(format!("{name} does not declare {hint} as a boolean"));
        }
    }
    let extra: Vec<&String> = annotations
        .keys()
        .filter(|key| *key != "title" && !HINTS.contains(&key.as_str()))
        .collect();
    if !extra.is_empty() {
        found.push(format!(
            "{name} declares annotations outside the spec: {extra:?}"
        ));
    }
    if annotations.get("readOnlyHint") == Some(&json!(true))
        && annotations.get("destructiveHint") == Some(&json!(true))
    {
        found.push(format!("{name} is both read-only and destructive"));
    }
    match row {
        None => found.push(format!("{name} has no row in tool-annotations.json")),
        Some(row) => {
            let expected = expected_annotations(row);
            if Value::Object(annotations.clone()) != expected {
                found.push(format!(
                    "{name} declares {} but the reviewed table says {expected}",
                    Value::Object(annotations.clone())
                ));
            }
        }
    }
    found
}

#[test]
fn every_tool_declares_a_title_and_the_four_hints_the_reviewed_table_gives_it() {
    let table = reviewed();
    let mut found = Vec::new();
    for (name, tool) in catalog() {
        found.extend(problems(&name, &tool, table.get(&name)));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_reviewed_table_names_only_tools_that_exist() {
    let tools = catalog();
    let stale: Vec<String> = reviewed()
        .into_keys()
        .filter(|name| !tools.contains_key(name))
        .collect();
    assert!(
        stale.is_empty(),
        "tool-annotations.json names tools the catalog does not have: {stale:?}"
    );
}

#[test]
fn every_reviewed_value_has_a_one_line_reason() {
    let mut found = Vec::new();
    for (name, row) in reviewed() {
        if row["title"].as_str().is_none_or(|t| t.trim().is_empty()) {
            found.push(format!("{name}: no title"));
        }
        for hint in HINTS {
            if !row[hint]["value"].is_boolean() {
                found.push(format!("{name}.{hint}: no boolean value"));
            }
            match row[hint]["reason"].as_str() {
                Some(reason) if !reason.trim().is_empty() && !reason.contains('\n') => {}
                _ => found.push(format!("{name}.{hint}: no one-line reason")),
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// The hints follow from one another the way the spec reads them: a tool that
/// changes nothing destroys nothing and can be repeated. `readOnlyHint` is the
/// same claim `READ_ONLY_TOOLS` makes to the audit layer, so the two cannot
/// disagree: a tool listed there writes no audit row of its own.
#[test]
fn read_only_hints_agree_with_the_servers_read_only_list() {
    let mut found = Vec::new();
    for (name, tool) in catalog() {
        let annotations = &tool["annotations"];
        let read_only = annotations["readOnlyHint"] == json!(true);
        if read_only != maidan_mcp::tools::is_read_only(&name) {
            found.push(format!(
                "{name}: readOnlyHint is {read_only} but READ_ONLY_TOOLS says {}",
                maidan_mcp::tools::is_read_only(&name)
            ));
        }
        if read_only && annotations["idempotentHint"] != json!(true) {
            found.push(format!("{name}: read-only but not idempotent"));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

fn read_verbs() -> BTreeSet<String> {
    fixture()["read_verbs"]
        .as_array()
        .expect("tool-annotations.json has read_verbs")
        .iter()
        .map(|verb| verb.as_str().expect("read verb").to_string())
        .collect()
}

/// Every problem with one tool's name. A model picks a tool by its name before
/// it reads the description, so the leading verb has to tell the truth: a read
/// verb never writes, and a tool that writes never starts with one. A new
/// read-only tool whose verb is not on the reviewed list fails until a reviewer
/// adds the verb, and no name bundles a read with a write.
fn name_problems(name: &str, read_only: bool, read_verbs: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    let snake = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !snake {
        found.push(format!("{name}: not snake_case"));
    }
    if name.len() > NAME_MAX {
        found.push(format!(
            "{name}: {} characters, over {NAME_MAX}",
            name.len()
        ));
    } else if CLAUDE_CODE_PREFIX.len() + name.len() > NAME_MAX {
        found.push(format!(
            "{name}: {CLAUDE_CODE_PREFIX}{name} is over {NAME_MAX} characters"
        ));
    }
    let verb = name.split('_').next().unwrap_or_default();
    match (read_verbs.contains(verb), read_only) {
        (true, false) => found.push(format!("{name}: `{verb}` is a read verb but it writes")),
        (false, true) => found.push(format!(
            "{name}: read-only, but `{verb}` is not in read_verbs"
        )),
        _ => {}
    }
    if name.contains("_or_") || name.contains("_and_") {
        found.push(format!("{name}: one tool, two verbs"));
    }
    found
}

#[test]
fn every_tool_name_fits_every_client_and_its_verb_says_whether_it_writes() {
    let read_verbs = read_verbs();
    let tools = catalog();
    let mut found = Vec::new();
    for name in tools.keys() {
        found.extend(name_problems(
            name,
            maidan_mcp::tools::is_read_only(name),
            &read_verbs,
        ));
    }
    let unused: Vec<&String> = read_verbs
        .iter()
        .filter(|verb| {
            !tools
                .keys()
                .any(|name| name.split('_').next() == Some(verb.as_str()))
        })
        .collect();
    if !unused.is_empty() {
        found.push(format!("read_verbs no tool uses: {unused:?}"));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_name_rules_refuse_the_names_they_exist_to_refuse() {
    let read_verbs = read_verbs();
    let refused =
        |name: &str, read_only: bool| !name_problems(name, read_only, &read_verbs).is_empty();
    assert!(refused(&"a".repeat(NAME_MAX + 1), false), "over 64");
    assert!(
        refused(&format!("set_{}", "x".repeat(NAME_MAX - 4 - 5)), false),
        "fits alone but not behind the Claude Code prefix"
    );
    assert!(refused("Get_thread", true), "not snake_case");
    assert!(refused("get-thread", true), "not snake_case");
    assert!(refused("get_thread", false), "a read verb that writes");
    assert!(
        refused("fetch_thread", true),
        "a read-only verb nobody reviewed"
    );
    assert!(refused("set_or_get_topic", false), "two verbs in one name");
    assert!(!refused("get_thread", true));
    assert!(!refused("set_topic", false));
}

/// The maintainer kept these as single tools on 2026-10-07 rather than split
/// them, on the condition that each is an honest write: never read-only, and
/// described so a model reading it knows the call changes something.
#[test]
fn the_mixed_tools_are_documented_writes() {
    let tools = catalog();
    let mixed = fixture()["mixed"]
        .as_object()
        .expect("tool-annotations.json has a mixed object")
        .clone();
    let mut found = Vec::new();
    for (name, row) in &mixed {
        let Some(tool) = tools.get(name) else {
            found.push(format!("{name}: listed as mixed but not in the catalog"));
            continue;
        };
        if maidan_mcp::tools::is_read_only(name) {
            found.push(format!("{name}: a mixed tool is a write, never read-only"));
        }
        if row["returns"].as_str().is_none_or(|r| r.trim().is_empty()) {
            found.push(format!("{name}: no account of what it returns"));
        }
        match row["writes"].as_str() {
            Some(writes) if !writes.trim().is_empty() => {
                let description = tool["description"].as_str().unwrap_or_default();
                if !description.contains(writes) {
                    found.push(format!("{name}: the description does not say `{writes}`"));
                }
            }
            _ => found.push(format!("{name}: no words naming its write")),
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

async fn mk_server() -> McpServer {
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
    McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
}

fn tools_list() -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/list".into(),
        params: json!({}),
    }
}

/// The annotations reach the client on every endpoint: the full surface and
/// both fixed profiles serve each tool with the annotations the table reviewed.
#[tokio::test]
async fn tools_list_serves_the_reviewed_annotations_on_every_endpoint() {
    let server = mk_server().await;
    let auth = AuthContext::bypass();
    let table = reviewed();

    let full = server.handle(tools_list(), &auth).await.result.unwrap();
    let mut served: BTreeSet<String> = BTreeSet::new();
    let mut found = Vec::new();
    for tool in full["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        served.insert(name.to_string());
        found.extend(problems(name, tool, table.get(name)));
    }
    assert_eq!(
        served,
        catalog().into_keys().collect::<BTreeSet<_>>(),
        "a bypass caller sees the whole catalog"
    );

    for profile in Profile::ALL {
        let listed = server
            .handle_profile(tools_list(), &auth, profile)
            .await
            .result
            .unwrap();
        let tools = listed["tools"].as_array().unwrap();
        assert_eq!(
            tools.len(),
            profile.tool_names().len(),
            "{}",
            profile.name()
        );
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            found.extend(
                problems(name, tool, table.get(name))
                    .into_iter()
                    .map(|problem| format!("{}: {problem}", profile.path())),
            );
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}
