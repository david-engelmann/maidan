//! A request that names a member names one of the caller's workspace.
//!
//! `PUT /threads/{id}/owner` with an `owner_id` that named no member answered
//! 500: the foreign key refused the row and the store's error surfaced as a
//! database error. One naming a member of another workspace was accepted.
//! This calls every documented operation whose body names a member, once with
//! an id that names no member and once with a member of another workspace, as
//! the owner of a seeded workspace, and requires a 4xx both times. The routes
//! that failed this answer 404, as a route answers for any referenced member
//! it cannot find, and the same for a foreign member, so the answer does not
//! say whether the id exists in another workspace.

mod seeded_workspace;

use maidan_types::{MemberKind, NewWorkspace};
use reqwest::Method;
use seeded_workspace::{
    example, fill, json_body_schema, member, query, seed_victim, spawn, Victim,
};
use serde_json::{json, Value};

/// The operations that answered 500 or accepted a foreign member.
const FIXED: &[&str] = &[
    "POST /messages/{id}/mentions",
    "POST /workspaces/{wid}/dm",
    "PUT /threads/{id}/assignee",
    "PUT /threads/{id}/owner",
];

/// The top-level body fields of `schema` that name a member, required or not.
fn member_fields(doc: &Value, schema: &Value, victim: &Victim) -> Vec<(String, bool)> {
    let schema = match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => &doc["components"]["schemas"][r.rsplit('/').next().unwrap()],
        None => schema,
    };
    let members = [victim.id("members"), victim.id("reviewers")];
    let mut out = Vec::new();
    for (name, prop) in schema["properties"].as_object().into_iter().flatten() {
        let is_array = prop["type"] == "array"
            || prop["type"]
                .as_array()
                .is_some_and(|t| t.contains(&json!("array")));
        let item = if is_array { &prop["items"] } else { prop };
        let uuid = item["format"] == "uuid"
            || item["anyOf"]
                .as_array()
                .into_iter()
                .chain(item["oneOf"].as_array())
                .flatten()
                .any(|o| o["format"] == "uuid");
        if uuid && members.contains(&victim.id_for_name(name)) {
            out.push((name.clone(), is_array));
        }
    }
    out
}

#[tokio::test]
async fn a_body_naming_an_unknown_or_foreign_member_is_refused_with_a_client_error() {
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
    let foreign = member(h.store.as_ref(), elsewhere, "foreigner", MemberKind::Human).await;
    let nobody = uuid::Uuid::now_v7();

    let mut probed = 0;
    let mut fixed_probed = std::collections::BTreeSet::new();
    let mut wrong = Vec::new();
    for (template, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let Ok(method) = method.to_uppercase().parse::<Method>() else {
                continue;
            };
            let Some(schema) = json_body_schema(op) else {
                continue;
            };
            let Some(path) = fill(template, &victim) else {
                continue;
            };
            let path = format!("{path}{}", query(&doc, op, &victim));
            for (field, is_array) in member_fields(&doc, schema, &victim) {
                for (who, id) in [
                    ("no member", nobody),
                    ("another workspace's member", foreign.0),
                ] {
                    let mut body = example(&doc, schema, "", &victim, 0);
                    body[&field] = if is_array { json!([id]) } else { json!(id) };
                    let res = h
                        .send(method.clone(), &path, &victim.bearer, Some(body))
                        .await;
                    let status = res.status();
                    probed += 1;
                    let label = format!("{method} {template}");
                    let expected = if FIXED.contains(&label.as_str()) {
                        fixed_probed.insert(label);
                        status == reqwest::StatusCode::NOT_FOUND
                    } else {
                        status.is_client_error()
                    };
                    if !expected {
                        let text = res.text().await.unwrap_or_default();
                        wrong.push(format!(
                            "{method} {template} with `{field}` naming {who}: {status} {}",
                            text.chars().take(160).collect::<String>()
                        ));
                    }
                }
            }
        }
    }
    println!("{probed} calls naming a member that is not the caller's");
    assert!(
        wrong.is_empty(),
        "a member id outside the caller's workspace was not refused:\n{}",
        wrong.join("\n")
    );
    assert_eq!(
        fixed_probed.iter().map(String::as_str).collect::<Vec<_>>(),
        FIXED,
        "a fixed route was not probed"
    );
    assert!(probed >= 18, "only {probed} calls probed");
}
