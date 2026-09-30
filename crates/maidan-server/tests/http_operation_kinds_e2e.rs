//! What each HTTP operation writes, checked against the kind
//! `contracts/http-operation-kinds.json` gives it.
//!
//! The seeded workspace of `tenant_isolation_e2e` is called again, this time
//! by its owner, whose token holds every capability. Every operation in the
//! served OpenAPI document is called once with a body built from its schema:
//!
//! - an operation classified `reads` that succeeds must leave every table as
//!   it found it. It may record its own access in the audit trail (an export,
//!   a resolved secret), but not a `mutation` row, which says a change
//!   happened. This is the check with teeth: the request layer records a
//!   successful POST, PUT, PATCH or DELETE that recorded nothing itself, but
//!   nothing records a GET, so a GET that writes is found only here.
//! - an operation classified `changes` that succeeds must leave an event
//!   attributed to the owner or an audit row naming the owner as actor.
//!
//! Only operations that answer 2xx are checked, and the test prints the rest
//! with their refusals. About 140 reads and 120 changes succeed. The others
//! need what this world does not have: a share-ticket credential (`/share`),
//! a peer credential (`/a2a/v1/events`), OIDC and a browser session
//! (`/auth`), the bootstrap routes, the S3 backend (multipart), the outbox
//! relay, an automation delivery, a held claim (renew, acknowledge, release,
//! usage), a `land_gate` skill, a third member (a group DM), or a query or
//! body the schema does not require but the handler does. The floors at the
//! end keep that set from growing unnoticed.
//!
//! Routes outside the OpenAPI document are not called: `/mcp`, whose tools
//! record per call; A2A and SCIM; the live streams; and the `/ui/api`
//! proxies, which reuse the handlers called here. Their kinds are checked
//! statically, by `http_operation_kinds_contract`, and by reading the
//! handlers.

mod seeded_workspace;

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use maidan_auth::ExportSigningKey;
use maidan_server::land_gate_advisor::{
    LandGateAdvice, LandGateAdviceRequest, LandGateAdviceThresholds, LandGateAdviceUsage,
    LandGateAdvisor, LandGateAdvisorError,
};
use maidan_server::{auth::MUTATION_ACTION, AppState};
use maidan_types::{LandColor, MemberId};
use reqwest::Method;
use seeded_workspace::{
    changed_tables, example, fill, json_body_schema, query, seed_victim, snapshot, spawn_with,
    Harness, Victim,
};
use serde_json::{json, Value};

/// Advice from nowhere, so the advice route can succeed.
struct FixedAdvisor;

#[async_trait::async_trait]
impl LandGateAdvisor for FixedAdvisor {
    async fn advise(
        &self,
        _request: LandGateAdviceRequest,
    ) -> Result<LandGateAdvice, LandGateAdvisorError> {
        Ok(LandGateAdvice {
            provider: "fixture".into(),
            model: "fixture".into(),
            raw_land: LandColor::Green,
            recommended_land: LandColor::Green,
            confidence: 0.95,
            probabilities: BTreeMap::from([("green".into(), 0.95)]),
            thresholds: LandGateAdviceThresholds {
                green_min_confidence: 0.9,
                red_min_confidence: 0.9,
            },
            usage: LandGateAdviceUsage {
                input_tokens: 1,
                output_tokens: 1,
            },
            latency_ms: 1,
        })
    }
}

const SIGNING_SEED: [u8; 32] = [0x2a; 32];

/// Operations that leave the owner unable to act, or the workspace gone, so
/// each runs in a world of its own after the rest.
const ENDS_THE_WORLD: &[&str] = &[
    "DELETE /channels/{cid}/members/{mid}",
    "DELETE /workspaces/{id}",
    "POST /members/{id}/freeze",
    "POST /workspaces/{id}/purge",
];

/// A legal hold refuses every deletion of workspace data while it stands.
const LIFT_HOLD: &str = "DELETE /workspaces/{id}/legal-holds/{hold_id}";

/// When each call runs: changes that add or set, then reads (so a read of
/// something set has something to read), then deletions, lifting the hold
/// first, then each call that ends the world.
fn phase(call: &Call, kind: &str) -> u8 {
    if ENDS_THE_WORLD.contains(&call.label.as_str()) {
        4
    } else if call.label == LIFT_HOLD {
        2
    } else if call.method == Method::DELETE && kind == "changes" {
        3
    } else if kind == "changes" {
        0
    } else {
        1
    }
}

async fn world() -> (Harness, Value, Victim) {
    let h = spawn_with(|state: &mut AppState| {
        let key = ExportSigningKey::from_seed(SIGNING_SEED);
        state.attach_export_verify_keys(vec![key.public_key_bytes()]);
        state.attach_export_signing(key);
        state.attach_land_gate_advisor(Arc::new(FixedAdvisor));
    })
    .await;
    let doc = serde_json::to_value(maidan_server::openapi::document()).unwrap();
    let victim = seed_victim(&h, &doc).await;
    (h, doc, victim)
}

/// Path templates with their parameter names dropped, so the router's
/// `{id}` and the document's `{wid}` for the same segment compare equal.
fn shape(path: &str) -> String {
    path.split('/')
        .map(|s| if s.starts_with('{') { "{}" } else { s })
        .collect::<Vec<_>>()
        .join("/")
}

fn kinds() -> BTreeMap<(String, String), String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        method: String,
        path: String,
        kind: String,
    }
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/http-operation-kinds.json");
    let entries: Vec<Entry> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    entries
        .into_iter()
        .map(|e| ((e.method, shape(&e.path)), e.kind))
        .collect()
}

async fn max_audit_id(h: &Harness) -> i64 {
    h.store
        .list_audit(1)
        .await
        .unwrap()
        .first()
        .map_or(0, |row| row.id)
}

/// The audit rows written after `after`.
async fn audit_after(h: &Harness, after: i64) -> Vec<maidan_types::AuditEvent> {
    let mut rows = h.store.list_audit(1000).await.unwrap();
    rows.retain(|row| row.id > after);
    rows
}

/// Whether an event after `after`, in any workspace or none, is attributed
/// to `actor`.
async fn attributed_event_after(h: &Harness, after: i64, actor: MemberId) -> bool {
    let last = h.store.max_event_id().await.unwrap();
    for id in after + 1..=last {
        let event = h.store.get_stored_event(id).await.unwrap();
        if event.attribution().is_some_and(|a| a.actor_id == actor) {
            return true;
        }
    }
    false
}

struct Call {
    label: String,
    method: Method,
    path: String,
    body: Option<Value>,
}

/// Each documented operation the seeded world can name, as its owner would
/// call it, with the kind the contract gives it.
fn calls(doc: &Value, victim: &Victim) -> Vec<(Call, String)> {
    let kinds = kinds();
    let mut out = Vec::new();
    let mut unclassified = Vec::new();
    for (template, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let Ok(method) = method.to_uppercase().parse::<Method>() else {
                continue;
            };
            if !matches!(
                method,
                Method::GET | Method::POST | Method::PUT | Method::PATCH | Method::DELETE
            ) {
                continue;
            }
            let label = format!("{method} {template}");
            let Some(kind) = kinds.get(&(method.to_string(), shape(template))) else {
                unclassified.push(label);
                continue;
            };
            let Some(path) = fill(template, victim) else {
                continue;
            };
            let body = match json_body_schema(op) {
                Some(schema) => Some(example(doc, schema, "", victim, 0)),
                None => {
                    matches!(method, Method::POST | Method::PUT | Method::PATCH).then(|| json!({}))
                }
            };
            let body = body.map(|mut b| {
                if template.contains("approval-gates") && b.get("action").is_some() {
                    b["action"] = json!("accept");
                }
                b
            });
            let path = format!("{path}{}", query(doc, op, victim));
            out.push((
                Call {
                    label,
                    method,
                    path,
                    body,
                },
                kind.clone(),
            ));
        }
    }
    assert!(
        unclassified.is_empty(),
        "documented operations with no kind in contracts/http-operation-kinds.json: {unclassified:?}"
    );
    out
}

/// A body the schema cannot describe: a real signed export to verify or
/// import.
async fn body_for(h: &Harness, victim: &Victim, call: &Call) -> Option<Value> {
    if call.label == "POST /workspaces/export/verify" || call.label == "POST /workspaces/import" {
        let export = h
            .send(
                Method::GET,
                &format!("/workspaces/{}/export", victim.ids["workspaces"]),
                &victim.bearer,
                None,
            )
            .await;
        assert!(export.status().is_success(), "export: {}", export.status());
        return Some(export.json().await.unwrap());
    }
    call.body.clone()
}

/// Lift the holds the calls before placed, so the deletions after can run.
async fn lift_every_hold(h: &Harness, victim: &Victim) {
    let ws = &victim.ids["workspaces"];
    let holds: Value = h
        .send(
            Method::GET,
            &format!("/workspaces/{ws}/legal-holds"),
            &victim.bearer,
            None,
        )
        .await
        .json()
        .await
        .unwrap();
    for hold in holds.as_array().into_iter().flatten() {
        if hold["lifted_at"].is_null() {
            let path = format!(
                "/workspaces/{ws}/legal-holds/{}",
                hold["id"].as_str().unwrap()
            );
            let res = h.send(Method::DELETE, &path, &victim.bearer, None).await;
            assert!(res.status().is_success(), "lift {path}: {}", res.status());
        }
    }
}

#[derive(Default)]
struct Tally {
    covered: Vec<String>,
    not_covered: Vec<String>,
    wrong: Vec<String>,
}

/// Call `call` and check that what it wrote matches `kind`.
async fn check(h: &Harness, victim: &Victim, call: &Call, kind: &str, tally: &mut Tally) {
    let owner = MemberId(victim.ids["members"].parse().unwrap());
    let body = body_for(h, victim, call).await;
    let tables = if kind == "reads" {
        Some(snapshot(&h.pool).await)
    } else {
        None
    };
    let audit_from = max_audit_id(h).await;
    let events_from = h.store.max_event_id().await.unwrap();

    let res = h
        .send(call.method.clone(), &call.path, &victim.bearer, body)
        .await;
    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["detail"].as_str().map(str::to_string))
            .unwrap_or(text);
        tally
            .not_covered
            .push(format!("{}: {status} {detail}", call.label));
        return;
    }
    // A streamed or large body must be read to the end before the call is over.
    let _ = res.bytes().await;
    tally.covered.push(call.label.clone());

    let audit = audit_after(h, audit_from).await;
    if let Some(before) = tables {
        let changed = changed_tables(&before, &snapshot(&h.pool).await);
        if !changed.is_empty() {
            tally.wrong.push(format!(
                "{} is classified reads but wrote to {}",
                call.label,
                changed.join(", ")
            ));
        }
        if audit.iter().any(|row| row.action == MUTATION_ACTION) {
            tally.wrong.push(format!(
                "{} is classified reads but was recorded as a change",
                call.label
            ));
        }
    } else {
        let audited = audit.iter().any(|row| row.actor_id == Some(owner));
        if !audited && !attributed_event_after(h, events_from, owner).await {
            tally.wrong.push(format!(
                "{} is classified changes but left no event or audit row naming who did it",
                call.label
            ));
        }
    }
}

#[tokio::test]
async fn every_successful_call_writes_what_its_kind_says() {
    let (h, doc, victim) = world().await;
    let mut all = calls(&doc, &victim);
    all.sort_by_key(|(call, kind)| phase(call, kind));

    let mut reads = Tally::default();
    let mut changes = Tally::default();
    let mut holds_lifted = false;
    for (call, kind) in &all {
        if phase(call, kind) == 3 && !holds_lifted {
            lift_every_hold(&h, &victim).await;
            holds_lifted = true;
        }
        let tally = if kind == "reads" {
            &mut reads
        } else {
            &mut changes
        };
        if ENDS_THE_WORLD.contains(&call.label.as_str()) {
            let (h, doc, victim) = world().await;
            let fresh = calls(&doc, &victim);
            let find = |label: &str| {
                fresh
                    .iter()
                    .find(|(c, _)| c.label == label)
                    .map(|(c, _)| c)
                    .unwrap()
            };
            let lift = find(LIFT_HOLD);
            let res = h
                .send(lift.method.clone(), &lift.path, &victim.bearer, None)
                .await;
            assert!(res.status().is_success(), "lift hold: {}", res.status());
            check(&h, &victim, find(&call.label), kind, tally).await;
        } else {
            check(&h, &victim, call, kind, tally).await;
        }
    }

    for (name, tally) in [("reads", &reads), ("changes", &changes)] {
        println!(
            "{name}: {} succeeded and were checked; {} did not succeed:",
            tally.covered.len(),
            tally.not_covered.len()
        );
        for line in &tally.not_covered {
            println!("  {line}");
        }
    }
    let wrong: Vec<_> = reads.wrong.iter().chain(&changes.wrong).collect();
    assert!(
        wrong.is_empty(),
        "operations whose writes disagree with contracts/http-operation-kinds.json:\n{}",
        wrong
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    for op in [
        "POST /threads/{id}/land-gate/advice",
        "POST /workspaces/export/verify",
        "POST /workspaces/{wid}/secrets/{name}/resolve",
    ] {
        assert!(
            reads.covered.iter().any(|c| c == op),
            "{op} is a read by POST and must be exercised"
        );
    }
    assert!(
        reads.covered.len() >= 135,
        "only {} reads checked",
        reads.covered.len()
    );
    assert!(
        changes.covered.len() >= 120,
        "only {} changes checked",
        changes.covered.len()
    );
}
