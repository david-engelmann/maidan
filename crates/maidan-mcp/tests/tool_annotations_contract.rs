//! Every MCP tool declares a title and the four behaviour hints of the MCP tool
//! spec (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`),
//! and each declared value matches `tests/fixtures/tool-annotations.json`, the
//! reviewed table that gives every value a reason a reviewer can check against
//! the handler. Directories reject a listing whose hints do not match what its
//! tools do, and a client asks before a destructive call only if the hint says
//! so; a new tool cannot ship without a reviewed row.

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

fn reviewed() -> BTreeMap<String, Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tool-annotations.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let table: Value = serde_json::from_str(&text).expect("tool-annotations.json");
    table["tools"]
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
