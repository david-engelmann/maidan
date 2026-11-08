//! Over MCP (#1703): a review seat with no usage is handed a review of its
//! own family's change. A `set_thread_result` whose envelope says that family
//! served it is refused; so is any result once `report_usage` (accepted)
//! names that family. Nothing is stored and the claim stays held for
//! `release_claim` with `decline: "own_family"`. A seat of another family
//! runs the identical flow and its verdict is stored. A member of another
//! workspace cannot write a verdict on the review at all.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability::THREAD_TRANSITION, AuthContext};
use maidan_mcp::{error::McpError, server::McpServer};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};

fn body(out: &Value) -> Value {
    let text = out["content"][0]["text"].as_str().expect("text content");
    serde_json::from_str(text).expect("json body")
}

/// The `report_usage` arguments a seat sends: zero tokens, with the
/// evidence naming its provider.
fn usage(thread: ThreadId, lease: &Value, model: &str, provider: &str) -> Value {
    json!({
        "thread_id": thread.0,
        "usage_report_id": uuid::Uuid::now_v7(),
        "claim_lease_id": lease,
        "model": model,
        "tokens": {
            "input": 0,
            "output": 0,
            "cache_read": 0,
            "cache_write_5m": 0,
            "cache_write_1h": 0,
        },
        "usd_micros": 0,
        "price_snapshot": {
            "input_usd_micros_per_million": 0,
            "output_usd_micros_per_million": 0,
            "cache_read_usd_micros_per_million": 0,
            "cache_write_5m_usd_micros_per_million": 0,
            "cache_write_1h_usd_micros_per_million": 0,
        },
        "turns": 1,
        "evidence": { "provider": provider, "harness": "test" },
    })
}

#[tokio::test]
async fn a_seat_whose_usage_names_the_authors_family_cannot_deliver_the_review() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );

    let room = |name: &'static str| {
        let store = store.clone();
        async move {
            let ws = store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .unwrap();
            let channel = store
                .create_channel(NewChannel {
                    workspace_id: ws.id,
                    name: "reviews".into(),
                    topic: None,
                    private: false,
                })
                .await
                .unwrap();
            (ws.id, channel.id)
        }
    };
    let member = |ws: WorkspaceId, handle: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id
        }
    };

    let (w, channel) = room("w").await;
    let (w2, _) = room("w2").await;
    let seat = member(w, "fresh-seat").await;
    let other = member(w, "other-seat").await;
    let stranger = member(w2, "fresh-seat").await;
    let review = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some("review".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id;
    store
        .set_thread_attributes(
            review,
            TaskAttributes {
                kind: TaskKind::Review,
                author_family: Some("openai".into()),
                ..TaskAttributes::default()
            },
        )
        .await
        .unwrap();
    let auth = |who: MemberId, ws: WorkspaceId| {
        AuthContext::from_session(who, ws, vec![THREAD_TRANSITION.to_string()])
    };
    let (seat_auth, other_auth, stranger_auth) =
        (auth(seat, w), auth(other, w), auth(stranger, w2));
    let claim = |who: &AuthContext| {
        let (server, who) = (&server, who.clone());
        async move {
            body(
                &server
                    .call_tool(
                        &who,
                        "claim_next_thread",
                        &json!({ "channel_id": channel.0, "lease_secs": 600 }),
                    )
                    .await
                    .unwrap(),
            )
        }
    };
    let verdict = json!({ "status": "done", "summary": "VERDICT: PASS" });
    let served_by = |provider: &str, model: &str| {
        json!({
            "schema": "maidan.waiter.result/1",
            "result_kind": "test.redteam/1",
            "status": "reviewed",
            "summary": "VERDICT: PASS",
            "rendered": "VERDICT: PASS",
            "served": { "provider": provider, "model": model },
        })
    };

    // The seat has reported nothing, so it is handed the openai review.
    let held = claim(&seat_auth).await;
    assert_eq!(held["id"], json!(review.0));
    let lease = held["claim_lease_id"].clone();

    // Its envelope says an openai model served it: refused before any usage.
    let declared = server
        .call_tool(
            &seat_auth,
            "set_thread_result",
            &json!({
                "thread_id": review.0,
                "claim_lease_id": lease,
                "result": served_by("openai", "gpt-5-codex"),
            }),
        )
        .await;
    assert!(
        matches!(&declared, Err(McpError::InvalidParams(m))
            if m.contains("says openai served it") && m.contains("decline own_family")),
        "{declared:?}"
    );

    let reported = server
        .call_tool(
            &seat_auth,
            "report_usage",
            &usage(review, &lease, "gpt-5-codex", "openai"),
        )
        .await
        .expect("the usage report is accepted");
    assert_eq!(body(&reported)["stopped"], json!(false));

    let refused = server
        .call_tool(
            &seat_auth,
            "set_thread_result",
            &json!({ "thread_id": review.0, "claim_lease_id": lease, "result": verdict }),
        )
        .await;
    assert!(
        matches!(&refused, Err(McpError::InvalidParams(m))
            if m.contains("usage names openai") && m.contains("decline own_family")),
        "{refused:?}"
    );
    assert!(
        store.get_thread_result(review).await.unwrap().is_none(),
        "nothing is stored, so nothing is delivered"
    );
    assert_eq!(
        store.get_thread(review).await.unwrap().assignee_id,
        Some(seat),
        "the claim stays held"
    );

    // Another tenant's member cannot reach the review's result at all.
    let foreign = server
        .call_tool(
            &stranger_auth,
            "set_thread_result",
            &json!({ "thread_id": review.0, "claim_lease_id": lease, "result": verdict }),
        )
        .await;
    assert!(
        foreign.is_err(),
        "another tenant is refused, got {foreign:?}"
    );

    server
        .call_tool(
            &seat_auth,
            "release_claim",
            &json!({ "thread_id": review.0, "claim_lease_id": lease, "decline": "own_family" }),
        )
        .await
        .expect("the seat declines it");

    // A seat of another family runs the identical flow, and its verdict is
    // stored.
    let taken = claim(&other_auth).await;
    assert_eq!(taken["id"], json!(review.0));
    let lease = taken["claim_lease_id"].clone();
    server
        .call_tool(
            &other_auth,
            "report_usage",
            &usage(review, &lease, "claude-opus-4-5", "anthropic"),
        )
        .await
        .expect("usage");
    let stored = server
        .call_tool(
            &other_auth,
            "set_thread_result",
            &json!({
                "thread_id": review.0,
                "claim_lease_id": lease,
                "result": served_by("anthropic", "claude-opus-4-5"),
            }),
        )
        .await
        .expect("another family's verdict is stored");
    assert_eq!(body(&stored)["produced_by"], json!(other.0));
    assert_eq!(
        store
            .get_thread_result(review)
            .await
            .unwrap()
            .expect("a result")
            .produced_by,
        other
    );
}
