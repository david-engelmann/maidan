//! `release_claim` with `decline: "own_family"` over MCP: a review seat with
//! no usage evidence lets go of a review of its own family's change, and
//! `claim_next_thread` hands it the next review instead of the same one,
//! while another member is still handed the declined review. A member of
//! another workspace cannot decline a thread it cannot read.

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

#[tokio::test]
async fn a_review_released_as_own_family_is_not_handed_back_to_that_seat() {
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
    let review = |channel: ChannelId, title: &'static str| {
        let store = store.clone();
        async move {
            let id = store
                .create_thread(NewThread {
                    channel_id: channel,
                    parent_thread_id: None,
                    title: Some(title.into()),
                    description: None,
                    created_by: None,
                })
                .await
                .unwrap()
                .id;
            store
                .set_thread_attributes(
                    id,
                    TaskAttributes {
                        kind: TaskKind::Review,
                        author_family: Some("openai".into()),
                        ..TaskAttributes::default()
                    },
                )
                .await
                .unwrap();
            id
        }
    };

    let (w, channel) = room("w").await;
    let (w2, _) = room("w2").await;
    let seat = member(w, "fresh-seat").await;
    let other = member(w, "other-seat").await;
    let stranger = member(w2, "fresh-seat").await;
    let first = review(channel, "first review").await;
    let second = review(channel, "second review").await;
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

    let held = claim(&seat_auth).await;
    assert_eq!(held["id"], json!(first.0));
    let lease = held["claim_lease_id"].clone();

    // An unknown reason is refused before anything moves.
    let unknown = server
        .call_tool(
            &seat_auth,
            "release_claim",
            &json!({ "thread_id": first.0, "claim_lease_id": lease, "decline": "busy" }),
        )
        .await;
    assert!(
        matches!(unknown, Err(McpError::InvalidParams(_))),
        "only a known decline is accepted, got {unknown:?}"
    );

    // Another tenant's member cannot reach the thread, decline or not.
    let foreign = server
        .call_tool(
            &stranger_auth,
            "release_claim",
            &json!({ "thread_id": first.0, "claim_lease_id": lease, "decline": "own_family" }),
        )
        .await;
    assert!(
        foreign.is_err(),
        "another tenant is refused, got {foreign:?}"
    );
    assert_eq!(
        store.get_thread(first).await.unwrap().assignee_id,
        Some(seat),
        "still held after both refusals"
    );

    let released = server
        .call_tool(
            &seat_auth,
            "release_claim",
            &json!({ "thread_id": first.0, "claim_lease_id": lease, "decline": "own_family" }),
        )
        .await
        .unwrap();
    assert_eq!(body(&released)["assignee_id"], Value::Null);

    // The seat is handed the next review, not the one it declined.
    let next = claim(&seat_auth).await;
    assert_eq!(next["id"], json!(second.0));
    server
        .call_tool(
            &seat_auth,
            "release_claim",
            &json!({
                "thread_id": second.0,
                "claim_lease_id": next["claim_lease_id"],
                "decline": "own_family",
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        claim(&seat_auth).await,
        Value::Null,
        "nothing left that the seat did not decline"
    );

    // Another member is still handed the declined review.
    let taken = claim(&other_auth).await;
    assert_eq!(taken["id"], json!(first.0));
}
