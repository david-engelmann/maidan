//! A member of another workspace is answered as a member that does not exist.
//!
//! Channel membership, reviewers and delegation grants refused another
//! workspace's member with `400 "member is not in this workspace"`, and an id
//! that named no member with 404: the difference told the caller the id was a
//! member somewhere. This calls every operation and MCP tool that takes a
//! member id, in the path or the body, as the owner of a seeded workspace,
//! once with an id that names no member and once with a member of another
//! workspace, and requires the two answers to be the same: status, problem
//! type and detail over HTTP, the JSON-RPC error or tool error over MCP.

mod seeded_workspace;

use maidan_types::{MemberKind, NewWorkspace};
use reqwest::Method;
use seeded_workspace::{
    example, fill, json_body_schema, member, query, seed_victim, spawn, Victim,
};
use serde_json::{json, Value};

const SENTINEL: &str = "00000000-0000-7000-8000-00000000abcd";

/// Probes that must be found, each with the answer both ids must get. The
/// rest of the probes are discovered from the OpenAPI document and the MCP
/// catalog and only have to answer the two ids alike; these are the ones that
/// answered differently, or that #1136 fixed, pinned so that a probe cannot
/// silently stop running.
const EXPECTED: &[(&str, &str)] = &[
    ("POST /channels/{cid}/members body `member_id`", NOT_FOUND),
    ("POST /threads/{id}/reviewers body `member_id`", NOT_FOUND),
    (
        "POST /workspaces/{wid}/delegation-grants body `delegate_id`",
        NOT_FOUND,
    ),
    (
        "POST /workspaces/{wid}/delegation-grants body `subject_id`",
        NOT_FOUND,
    ),
    (
        "POST /workspaces/{wid}/group-dms body `member_ids`",
        NOT_FOUND,
    ),
    (
        "GET /workspaces/{wid}/members/{mid}/tokens path {mid}",
        NOT_FOUND,
    ),
    (
        "POST /workspaces/{wid}/members/{mid}/tokens path {mid}",
        NOT_FOUND,
    ),
    ("GET /members/{id} path {id}", NOT_FOUND),
    ("POST /members/{id}/freeze path {id}", NOT_FOUND),
    ("GET /members/{id}/wip path {id}", NOT_FOUND),
    (
        "POST /members/{id}/member-follows body `followed_member_id`",
        NOT_FOUND,
    ),
    ("PUT /threads/{id}/owner body `owner_id`", NOT_FOUND),
    ("PUT /threads/{id}/assignee body `assignee_id`", NOT_FOUND),
    ("POST /messages/{id}/mentions body `member_id`", NOT_FOUND),
    (
        "POST /workspaces/{wid}/dm body `other_member_id`",
        NOT_FOUND,
    ),
    ("MCP add_channel_member `member_id`", MCP_NOT_FOUND),
    ("MCP add_reviewer `member_id`", MCP_NOT_FOUND),
    ("MCP create_delegation_grant `delegate_id`", MCP_NOT_FOUND),
    ("MCP create_delegation_grant `subject_id`", MCP_NOT_FOUND),
    ("MCP freeze_member `member_id`", MCP_NOT_FOUND),
    ("MCP unfreeze_member `member_id`", MCP_NOT_FOUND),
    ("MCP follow_member `followed_member_id`", MCP_NOT_FOUND),
    ("MCP get_member_occupancy `member_id`", MCP_NOT_FOUND),
    ("MCP get_member_wip `member_id`", MCP_NOT_FOUND),
    ("MCP list_assigned_threads `member_id`", MCP_NOT_FOUND),
    ("MCP set_thread_owner `owner_id`", MCP_NOT_FOUND),
    ("MCP assign_thread `assignee_id`", MCP_NOT_FOUND),
    ("MCP record_mention `member_id`", MCP_NOT_FOUND),
    ("MCP open_dm_conversation `other_member_id`", MCP_NOT_FOUND),
];

const NOT_FOUND: &str =
    "404 Not Found https://maidan.dev/problems/not-found the requested resource does not exist";
const MCP_NOT_FOUND: &str = "error -32004 \"resource not found\"";

/// A request with [`SENTINEL`] where a member id goes.
struct Probe {
    label: String,
    method: Method,
    path: String,
    body: Option<Value>,
}

/// The ids of the victim's members.
fn member_ids(victim: &Victim) -> Vec<&str> {
    ["members", "reviewers"]
        .iter()
        .filter_map(|seg| victim.id(seg))
        .collect()
}

fn resolve<'a>(doc: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => resolve(
            doc,
            &doc["components"]["schemas"][r.rsplit('/').next().unwrap()],
        ),
        None => schema,
    }
}

/// Whether a field or parameter called `name` takes a member id. `author`
/// is the search filter's name for one.
fn names_member(victim: &Victim, name: &str) -> bool {
    let members = member_ids(victim);
    name == "author"
        || victim
            .id_for_name(name)
            .is_some_and(|id| members.contains(&id))
}

fn is_uuid(doc: &Value, schema: &Value) -> bool {
    let schema = resolve(doc, schema);
    schema["format"] == "uuid"
        || schema["anyOf"]
            .as_array()
            .into_iter()
            .chain(schema["oneOf"].as_array())
            .flatten()
            .any(|o| resolve(doc, o)["format"] == "uuid")
}

/// The top-level fields of `schema` that name a member, and whether each is
/// an array.
fn member_fields(doc: &Value, schema: &Value, victim: &Victim) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for (name, prop) in resolve(doc, schema)["properties"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let prop = resolve(doc, prop);
        let is_array = prop["type"] == "array"
            || prop["type"]
                .as_array()
                .is_some_and(|t| t.contains(&json!("array")));
        let item = if is_array { &prop["items"] } else { prop };
        if is_uuid(doc, item) && names_member(victim, name) {
            out.push((name.clone(), is_array));
        }
    }
    out
}

fn http_probes(doc: &Value, victim: &Victim) -> Vec<Probe> {
    let members = member_ids(victim);
    let mut probes = Vec::new();
    for (template, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let Ok(method) = method.to_uppercase().parse::<Method>() else {
                continue;
            };
            let Some(path) = fill(template, victim) else {
                continue;
            };
            let query = query(doc, op, victim);
            let schema = json_body_schema(op);
            let body = schema.map(|s| example(doc, s, "", victim, 0));
            let label = format!("{method} {template}");

            let names: Vec<&str> = template.split('/').collect();
            let filled: Vec<&str> = path.split('/').collect();
            for (i, name) in names.iter().enumerate() {
                if name.starts_with('{') && members.contains(&filled[i]) {
                    let mut segments = filled.clone();
                    segments[i] = SENTINEL;
                    probes.push(Probe {
                        label: format!("{label} path {name}"),
                        method: method.clone(),
                        path: format!("{}{query}", segments.join("/")),
                        body: body.clone(),
                    });
                }
            }
            for param in op["parameters"].as_array().into_iter().flatten() {
                let param = resolve(doc, param);
                let name = param["name"].as_str().unwrap_or_default();
                if param["in"] == "query"
                    && is_uuid(doc, &param["schema"])
                    && names_member(victim, name)
                {
                    let mut pairs: Vec<String> = query
                        .trim_start_matches('?')
                        .split('&')
                        .filter(|p| !p.is_empty() && !p.starts_with(&format!("{name}=")))
                        .map(str::to_string)
                        .collect();
                    pairs.push(format!("{name}={SENTINEL}"));
                    probes.push(Probe {
                        label: format!("{label} query `{name}`"),
                        method: method.clone(),
                        path: format!("{path}?{}", pairs.join("&")),
                        body: body.clone(),
                    });
                }
            }
            let Some(schema) = schema else {
                continue;
            };
            for (field, is_array) in member_fields(doc, schema, victim) {
                let mut body = example(doc, schema, "", victim, 0);
                body[&field] = if is_array {
                    // The caller's own members first, so a list with a
                    // minimum length reaches the member check.
                    json!([members.clone(), vec![SENTINEL]].concat())
                } else {
                    json!(SENTINEL)
                };
                probes.push(Probe {
                    label: format!("{label} body `{field}`"),
                    method: method.clone(),
                    path: format!("{path}{query}"),
                    body: Some(body),
                });
            }
        }
    }
    probes
}

/// Each MCP tool argument that names a member, with the tool's other
/// required arguments filled from the victim.
fn mcp_probes(doc: &Value, tools: &[Value], victim: &Victim) -> Vec<(String, Value)> {
    let members = member_ids(victim);
    let mut probes = Vec::new();
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let schema = &tool["inputSchema"];
        for (field, prop) in schema["properties"].as_object().into_iter().flatten() {
            let is_array = prop["type"] == "array";
            let item = if is_array { &prop["items"] } else { prop };
            if item["type"] != "string" && item.get("type").is_some() {
                continue;
            }
            if !names_member(victim, field) {
                continue;
            }
            let mut args = example(doc, schema, "", victim, 0);
            args[field] = if is_array {
                json!([members.clone(), vec![SENTINEL]].concat())
            } else {
                json!(SENTINEL)
            };
            probes.push((
                format!("MCP {name} `{field}`"),
                json!({ "name": name, "arguments": args }),
            ));
        }
    }
    probes
}

/// What an MCP answer says, with the probed id taken out.
fn mcp_answer(res: &Value, id: &str) -> String {
    let text = res.to_string().replace(id, "{member}");
    let res: Value = serde_json::from_str(&text).unwrap();
    if let Some(error) = res.get("error") {
        format!("error {} {}", error["code"], error["message"])
    } else if res["result"]["isError"] == true {
        format!("tool error {}", res["result"]["content"][0]["text"])
    } else {
        format!("ok {}", text.chars().take(160).collect::<String>())
    }
}

fn with_id(value: &Value, id: &str) -> Value {
    serde_json::from_str(&value.to_string().replace(SENTINEL, id)).unwrap()
}

/// What an answer says, with the probed id taken out.
fn answer(status: reqwest::StatusCode, text: &str, id: &str) -> String {
    let text = text.replace(id, "{member}");
    match serde_json::from_str::<Value>(&text) {
        Ok(problem) if problem.get("type").is_some() => format!(
            "{status} {} {}",
            problem["type"].as_str().unwrap_or_default(),
            problem["detail"].as_str().unwrap_or_default()
        ),
        _ => format!("{status} {}", text.chars().take(160).collect::<String>()),
    }
}

#[tokio::test]
async fn a_foreign_member_is_answered_as_no_member_on_every_route() {
    let h = spawn().await;
    let doc = serde_json::to_value(maidan_server::openapi::document()).unwrap();
    let victim = seed_victim(&h, &doc).await;
    let elsewhere = h
        .store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .unwrap()
        .id;
    let foreign = member(h.store.as_ref(), elsewhere, "foreigner", MemberKind::Human)
        .await
        .0
        .to_string();

    let probes = http_probes(&doc, &victim);
    let mut differ = Vec::new();
    let mut all = Vec::new();
    let mut seen = std::collections::BTreeMap::new();
    for probe in &probes {
        let mut answers = Vec::new();
        for id in [uuid::Uuid::now_v7().to_string(), foreign.clone()] {
            let res = h
                .send(
                    probe.method.clone(),
                    &probe.path.replace(SENTINEL, &id),
                    &victim.bearer,
                    probe.body.as_ref().map(|b| with_id(b, &id)),
                )
                .await;
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            answers.push(answer(status, &text, &id));
        }
        all.push(format!("{}: {} | {}", probe.label, answers[0], answers[1]));
        seen.insert(probe.label.clone(), answers.clone());
        if answers[0] != answers[1] {
            differ.push(format!(
                "{}\n  no member:        {}\n  foreign member:   {}",
                probe.label, answers[0], answers[1]
            ));
        }
    }
    let listed: Value = h
        .send(
            Method::POST,
            "/mcp",
            &victim.bearer,
            Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
        )
        .await
        .json()
        .await
        .unwrap();
    let tools = listed["result"]["tools"].as_array().unwrap().clone();
    for (label, params) in mcp_probes(&doc, &tools, &victim) {
        let mut answers = Vec::new();
        for id in [uuid::Uuid::now_v7().to_string(), foreign.clone()] {
            let res: Value = h
                .send(
                    Method::POST,
                    "/mcp",
                    &victim.bearer,
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": with_id(&params, &id),
                    })),
                )
                .await
                .json()
                .await
                .unwrap();
            answers.push(mcp_answer(&res, &id));
        }
        all.push(format!("{label}: {} | {}", answers[0], answers[1]));
        seen.insert(label.clone(), answers.clone());
        if answers[0] != answers[1] {
            differ.push(format!(
                "{label}\n  no member:        {}\n  foreign member:   {}",
                answers[0], answers[1]
            ));
        }
    }
    println!("{}", all.join("\n"));
    let mut wrong = Vec::new();
    for (label, expected) in EXPECTED {
        match seen.get(*label) {
            None => wrong.push(format!("{label}: not probed")),
            Some(answers) if answers.iter().any(|a| a != expected) => {
                wrong.push(format!("{label}: {answers:?}, expected {expected}"))
            }
            Some(_) => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "a member-taking operation did not answer as expected:\n{}",
        wrong.join("\n")
    );
    assert!(seen.len() >= 100, "only {} probes", seen.len());
    assert!(
        differ.is_empty(),
        "a foreign member is answered differently from no member:\n{}",
        differ.join("\n")
    );
}
