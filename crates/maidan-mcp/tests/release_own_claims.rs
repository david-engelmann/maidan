//! `release_own_claims` frees the calling member's live claims in its own
//! workspace and nothing else: not another member's, not the same agent's
//! claims under its member in another workspace, and never a workspace the
//! token was not minted for. The old process is fenced out afterwards.

use std::collections::BTreeSet;
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability::THREAD_TRANSITION, AuthContext};
use maidan_mcp::{error::McpError, server::McpServer};
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;
use serde_json::{json, Value};

const LEASE_SECS: i64 = 600;

fn released(out: &Value) -> BTreeSet<String> {
    let text = out["content"][0]["text"].as_str().expect("text content");
    let body: Value = serde_json::from_str(text).expect("json body");
    body["released"]
        .as_array()
        .expect("released is a list")
        .iter()
        .map(|id| id.as_str().expect("a thread id").to_string())
        .collect()
}

#[tokio::test]
async fn a_restarted_claimant_releases_only_its_own_claims_in_its_own_workspace() {
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

    let workspace = |name: &'static str| {
        let store = store.clone();
        async move {
            let ws = store
                .create_workspace(NewWorkspace { name: name.into() })
                .await
                .unwrap();
            let channel = store
                .create_channel(NewChannel {
                    workspace_id: ws.id,
                    name: "queue".into(),
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
    // Distinct titles: identical ones are fingerprinted as duplicates of the
    // first, and a duplicate is withheld from claim_next.
    let task = |channel: ChannelId, title: &'static str| {
        let store = store.clone();
        async move {
            store
                .create_thread(NewThread {
                    channel_id: channel,
                    parent_thread_id: None,
                    title: Some(title.into()),
                    description: None,
                    created_by: None,
                })
                .await
                .unwrap();
        }
    };
    let claim = |channel: ChannelId, who: MemberId| {
        let store = store.clone();
        async move {
            store
                .claim_next_thread(channel, who, Some(LEASE_SECS), None)
                .await
                .unwrap()
                .expect("a thread is claimable")
        }
    };

    let (w, w_channel) = workspace("w").await;
    let (w2, w2_channel) = workspace("w2").await;
    let a = member(w, "a").await;
    let b = member(w, "b").await;
    // The same agent's member in the second workspace: a member belongs to
    // one workspace, so this is how "A" holds work in W2.
    let a_in_w2 = member(w2, "a").await;
    for title in ["first", "second", "third"] {
        task(w_channel, title).await;
    }
    task(w2_channel, "elsewhere").await;
    let a_first = claim(w_channel, a).await;
    let a_second = claim(w_channel, a).await;
    let b_claim = claim(w_channel, b).await;
    let a_w2_claim = claim(w2_channel, a_in_w2).await;

    let auth_a = AuthContext::from_session(a, w, vec![THREAD_TRANSITION.to_string()]);
    let release = |ws: WorkspaceId| {
        let (server, auth) = (&server, &auth_a);
        async move {
            server
                .call_tool(auth, "release_own_claims", &json!({ "workspace_id": ws.0 }))
                .await
        }
    };

    // A workspace the token was not minted for is refused, and nothing moves.
    let refused = release(w2).await;
    assert!(
        matches!(refused, Err(McpError::Forbidden(_))),
        "another workspace must be refused, got {refused:?}"
    );
    assert_eq!(
        store.get_thread(a_w2_claim.id).await.unwrap().assignee_id,
        Some(a_in_w2)
    );

    let out = release(w).await.unwrap();
    let expected: BTreeSet<String> = [a_first.id, a_second.id]
        .iter()
        .map(|id| id.0.to_string())
        .collect();
    assert_eq!(released(&out), expected, "exactly a's two claims in w");
    for id in [a_first.id, a_second.id] {
        let thread = store.get_thread(id).await.unwrap();
        assert_eq!(thread.assignee_id, None, "back in the queue");
        assert_eq!(thread.claim_lease_id, None);
    }
    assert_eq!(
        store.get_thread(b_claim.id).await.unwrap().assignee_id,
        Some(b),
        "b's claim is untouched"
    );
    let kept = store.get_thread(a_w2_claim.id).await.unwrap();
    assert_eq!(kept.assignee_id, Some(a_in_w2), "the w2 claim is untouched");
    assert_eq!(kept.claim_lease_id, a_w2_claim.claim_lease_id);

    // A second call has nothing left to release.
    assert!(released(&release(w).await.unwrap()).is_empty());

    // The old process's lease is dead: its renew is refused.
    let renew = server
        .call_tool(
            &auth_a,
            "renew_claim",
            &json!({
                "thread_id": a_first.id.0,
                "claim_lease_id": a_first.claim_lease_id.unwrap().0,
            }),
        )
        .await;
    assert!(
        matches!(renew, Err(McpError::NotFound)),
        "the stale lease must not renew, got {renew:?}"
    );

    // A note longer than the cap is refused before anything is released.
    let long = server
        .call_tool(
            &auth_a,
            "release_own_claims",
            &json!({ "workspace_id": w.0, "note": "x".repeat(2001) }),
        )
        .await;
    assert!(
        matches!(long, Err(McpError::InvalidParams(_))),
        "an over-long note must be refused, got {long:?}"
    );
}
