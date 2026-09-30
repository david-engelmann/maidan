//! What each A2A method writes, checked against `Method::changes`.
//!
//! One JSON-RPC endpoint serves every A2A method, and a JSON-RPC error still
//! answers HTTP 200, so the request layer cannot tell a read from a change:
//! it recorded `ListTasks` as one. A2A records per method instead
//! (`a2a_agent::recorded`). This calls every method in `Method::ALL` as a
//! member of a seeded workspace:
//!
//! - a method that does not change must leave every table as it found it and
//!   write no audit row;
//! - a method that changes must leave an event attributed to the caller or an
//!   audit row naming the caller as actor.
//!
//! A new method does not compile until `Method::changes` classifies it, and
//! fails here until it has a request below.

mod seeded_workspace;

use futures::StreamExt;
use maidan_a2a::Method;
use maidan_auth::capability;
use maidan_server::auth::MUTATION_ACTION;
use maidan_store::A2aTaskWrite;
use maidan_types::{MemberId, MemberKind, NewWorkspace, WorkspaceId};
use seeded_workspace::{changed_tables, member, snapshot, spawn_with, token, Harness};
use serde_json::{json, Value};

const TASK: &str = "task-kinds";
const CONFIG: &str = "config-kinds";

struct World {
    h: Harness,
    ws: WorkspaceId,
    caller: MemberId,
    bearer: String,
}

async fn world() -> World {
    let h = spawn_with(|state| {
        state.a2a_card.public_origin = Some("https://maidan.example".into());
    })
    .await;
    let ws = h
        .store
        .create_workspace(NewWorkspace {
            name: "a2a-kinds".into(),
        })
        .await
        .unwrap()
        .id;
    let caller = member(h.store.as_ref(), ws, "a2a-caller", MemberKind::Agent).await;
    let bearer = token(
        h.store.as_ref(),
        ws,
        caller,
        [
            capability::WORKSPACE_READ,
            capability::WORKSPACE_WRITE,
            capability::MESSAGE_POST,
        ]
        .iter()
        .map(|c| c.to_string())
        .collect(),
    )
    .await;
    // A task that has not finished, which SendMessage never produces, so
    // there is something to subscribe to and cancel.
    h.store
        .upsert_a2a_task(A2aTaskWrite {
            workspace_id: ws,
            task_id: TASK,
            context_id: Some("ctx-kinds"),
            thread_id: None,
            state: "TASK_STATE_WORKING",
            status_at: chrono::Utc::now(),
            task_json: json!({
                "id": TASK,
                "contextId": "ctx-kinds",
                "status": { "state": "TASK_STATE_WORKING" },
            }),
        })
        .await
        .unwrap();
    World {
        h,
        ws,
        caller,
        bearer,
    }
}

fn message(text: &str) -> Value {
    json!({
        "messageId": uuid::Uuid::now_v7().to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": text }],
    })
}

/// The request each method is called with, and when it runs: what creates
/// first, then the reads, then what deletes or ends the task.
fn request(method: Method) -> (u8, Value) {
    match method {
        Method::SendMessage | Method::SendStreamingMessage => {
            (0, json!({ "message": message(method.name()) }))
        }
        Method::CreatePushNotificationConfig => (
            0,
            json!({ "taskId": TASK, "id": CONFIG, "url": "https://hooks.example/a2a" }),
        ),
        Method::GetTask | Method::SubscribeToTask => (1, json!({ "id": TASK })),
        Method::ListTasks | Method::GetExtendedAgentCard => (1, json!({})),
        Method::GetPushNotificationConfig => (1, json!({ "taskId": TASK, "id": CONFIG })),
        Method::ListPushNotificationConfigs => (1, json!({ "taskId": TASK })),
        Method::DeletePushNotificationConfig => (2, json!({ "taskId": TASK, "id": CONFIG })),
        Method::CancelTask => (3, json!({ "id": TASK })),
    }
}

impl World {
    /// Call `method` over JSON-RPC and return the first JSON-RPC response it
    /// answers with, whether a plain body or the first frame of a stream.
    async fn rpc(&self, method: &str, params: Value) -> Value {
        let res = self
            .h
            .client
            .post(self.h.url("/a2a/v1/rpc"))
            .bearer_auth(&self.bearer)
            .header("A2A-Version", "1.0")
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "{method}");
        let streamed = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if !streamed {
            return res.json().await.unwrap();
        }
        let mut body = res.bytes_stream();
        let mut text = String::new();
        while let Some(chunk) = body.next().await {
            text.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            if let Some(line) = text.lines().find(|l| l.starts_with("data:")) {
                if text.contains("\n\n") {
                    return serde_json::from_str(line.trim_start_matches("data:").trim()).unwrap();
                }
            }
        }
        panic!("{method}: the stream ended before its first frame");
    }

    async fn last_audit_id(&self) -> i64 {
        self.h
            .store
            .list_audit(1)
            .await
            .unwrap()
            .first()
            .map_or(0, |row| row.id)
    }

    async fn audit_after(&self, after: i64) -> Vec<maidan_types::AuditEvent> {
        let mut rows = self.h.store.list_audit(1000).await.unwrap();
        rows.retain(|row| row.id > after);
        rows
    }

    async fn attributed_event_after(&self, after: i64) -> bool {
        let last = self.h.store.max_event_id().await.unwrap();
        for id in after + 1..=last {
            let event = self.h.store.get_stored_event(id).await.unwrap();
            if event
                .attribution()
                .is_some_and(|a| a.actor_id == self.caller)
            {
                return true;
            }
        }
        false
    }
}

#[tokio::test]
async fn every_a2a_method_writes_what_its_kind_says() {
    let w = world().await;
    let mut methods: Vec<(Method, u8, Value)> = Method::ALL
        .iter()
        .map(|m| {
            let (phase, params) = request(*m);
            (*m, phase, params)
        })
        .collect();
    methods.sort_by_key(|(_, phase, _)| *phase);

    let mut wrong = Vec::new();
    for (method, _, params) in methods {
        let before = snapshot(&w.h.pool).await;
        let audit_from = w.last_audit_id().await;
        let events_from = w.h.store.max_event_id().await.unwrap();

        let changes = method.changes();
        let method = method.name();
        let resp = w.rpc(method, params).await;
        assert!(
            resp.get("error").is_none(),
            "{method} must succeed to be checked: {resp}"
        );

        let audit = w.audit_after(audit_from).await;
        if !changes {
            let changed = changed_tables(&before, &snapshot(&w.h.pool).await);
            if !changed.is_empty() {
                wrong.push(format!(
                    "{method} is classified a read but wrote to {}",
                    changed.join(", ")
                ));
            }
            if !audit.is_empty() {
                wrong.push(format!(
                    "{method} is classified a read but wrote audit rows {:?}",
                    audit.iter().map(|r| &r.action).collect::<Vec<_>>()
                ));
            }
        } else {
            let audited = audit.iter().any(|row| row.actor_id == Some(w.caller));
            if !audited && !w.attributed_event_after(events_from).await {
                wrong.push(format!(
                    "{method} changes state but left no event or audit row naming its caller"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(Method::ALL.iter().any(|m| m.changes()));
    assert!(Method::ALL.iter().any(|m| !m.changes()));
}

/// A change that writes no event of its own gets a `mutation` row naming the
/// A2A method, not the endpoint it came through.
#[tokio::test]
async fn a_change_that_records_nothing_itself_is_recorded_under_its_method() {
    let w = world().await;
    let from = w.last_audit_id().await;
    let (_, params) = request(Method::CreatePushNotificationConfig);
    let resp = w.rpc("CreateTaskPushNotificationConfig", params).await;
    assert!(resp.get("error").is_none(), "{resp}");
    let audit = w.audit_after(from).await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    let row = &audit[0];
    assert_eq!(row.action, MUTATION_ACTION);
    assert_eq!(row.actor_id, Some(w.caller));
    assert_eq!(row.target_id, Some(w.ws.0));
    assert_eq!(row.metadata["surface"], "a2a");
    assert_eq!(row.metadata["binding"], "jsonrpc");
    assert_eq!(
        row.metadata["operation"],
        "CreateTaskPushNotificationConfig"
    );
}

/// A JSON-RPC error answers HTTP 200, which the request layer read as a
/// successful change. A refused call changes nothing and records nothing.
#[tokio::test]
async fn a_json_rpc_error_records_nothing() {
    let w = world().await;
    let calls = [
        ("GetTask", json!({ "id": "no-such-task" })),
        ("CancelTask", json!({ "id": "no-such-task" })),
        (
            "CreateTaskPushNotificationConfig",
            json!({ "taskId": "no-such-task", "url": "https://hooks.example/a2a" }),
        ),
        ("SendMessage", json!({ "message": "not a message" })),
        ("NoSuchMethod", json!({})),
    ];
    for (method, params) in calls {
        let before = snapshot(&w.h.pool).await;
        let from = w.last_audit_id().await;
        let resp = w.rpc(method, params).await;
        assert!(resp.get("error").is_some(), "{method} should fail: {resp}");
        assert_eq!(
            changed_tables(&before, &snapshot(&w.h.pool).await),
            Vec::<String>::new(),
            "{method}"
        );
        let audit = w.audit_after(from).await;
        assert!(audit.is_empty(), "{method} wrote {audit:?}");
    }
}

/// The HTTP+JSON binding classifies by method too: `POST /tasks/{id}:subscribe`
/// is a read, `POST /tasks/{id}:cancel` a change.
#[tokio::test]
async fn the_rest_binding_records_by_method_not_by_http_method() {
    let w = world().await;
    let rest = |method: reqwest::Method, path: String| {
        w.h.client
            .request(method, w.h.url(&format!("/a2a/v1{path}")))
            .bearer_auth(&w.bearer)
            .header("A2A-Version", "1.0")
            .send()
    };

    let from = w.last_audit_id().await;
    let res = rest(reqwest::Method::POST, format!("/tasks/{TASK}:subscribe"))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let mut body = res.bytes_stream();
    let first = body.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains(TASK));
    drop(body);
    assert!(w.audit_after(from).await.is_empty());

    let from = w.last_audit_id().await;
    let res = rest(reqwest::Method::POST, format!("/tasks/{TASK}:cancel"))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let audit = w.audit_after(from).await;
    assert!(
        audit.iter().any(|row| row.action == MUTATION_ACTION
            && row.actor_id == Some(w.caller)
            && row.metadata["binding"] == "rest"
            && row.metadata["operation"] == "CancelTask"),
        "{audit:?}"
    );
}
