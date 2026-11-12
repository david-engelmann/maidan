//! Thread results: set, get, wait, aggregate, lineage, and the refusals.

use super::*;

#[tokio::test]
async fn result_tools_set_get_wait_and_aggregate() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_bus::InMemoryBus;
    use std::time::Duration;

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
    let ws = store
        .create_workspace(NewWorkspace { name: "res".into() })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let mk = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
        created_by: None,
    };
    // parent depends on dep1 + dep2.
    let parent = store.create_thread(mk("parent")).await.unwrap();
    let dep1 = store.create_thread(mk("dep1")).await.unwrap();
    let dep2 = store.create_thread(mk("dep2")).await.unwrap();
    store
        .add_thread_dependency(parent.id, dep1.id)
        .await
        .unwrap();
    store
        .add_thread_dependency(parent.id, dep2.id)
        .await
        .unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    )
    .with_event_bus(Arc::new(InMemoryBus::new()));
    // A real session member so `produced_by` (a NOT-NULL FK) resolves — the
    // bypass nil member would FK-fail set_thread_result.
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    // No result yet → null.
    let miss = server
        .call_tool(
            &auth,
            "get_thread_result",
            &json!({ "thread_id": dep1.id.0 }),
        )
        .await
        .unwrap();
    assert!(unwrap_content(miss).is_null());

    // Set dep1's result, then read it back.
    let payload = json!({ "answer": 42, "ok": true });
    let lease = server
        .store
        .claim_lease_for(dep1.id, agent.id)
        .await
        .unwrap();
    let set = server
        .call_tool(
            &auth,
            "set_thread_result",
            &json!({ "thread_id": dep1.id.0, "result": payload, "claim_lease_id": lease.0 }),
        )
        .await
        .unwrap();
    let set = unwrap_content(set);
    assert_eq!(set["result"], payload);
    assert_eq!(set["produced_by"], json!(agent.id.0));
    let got = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_thread_result",
                &json!({ "thread_id": dep1.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(got["result"], payload);

    // Wait_for_result with since_log_id catches a result SET before the
    // call subscribes. dep1's result was just set, so a lookback from the
    // head of the log finds its thread_result_set and returns the payload
    // without waiting on a live event.
    let looked = unwrap_content(
        server
            .call_tool(
                &auth,
                "wait_for_result",
                &json!({ "thread_id": dep1.id.0, "timeout_ms": 300, "since_log_id": 0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        looked["result"], payload,
        "lookback returns dep1's already-set result"
    );

    // wait_for_result on dep2 wakes when its result is produced after subscribe.
    let dep2_payload = json!({ "answer": 7 });
    let wait_args = json!({ "thread_id": dep2.id.0, "timeout_ms": 5000 });
    let dep2_lease = server
        .store
        .claim_lease_for(dep2.id, agent.id)
        .await
        .unwrap();
    let set_args = json!({
        "thread_id": dep2.id.0,
        "claim_lease_id": dep2_lease.0,
        "result": dep2_payload,
    });
    let (out, _) = tokio::join!(
        server.call_tool(&auth, "wait_for_result", &wait_args),
        async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            server
                .call_tool(&auth, "set_thread_result", &set_args)
                .await
                .unwrap();
        }
    );
    let woke = unwrap_content(out.unwrap());
    assert_eq!(
        woke["result"], dep2_payload,
        "wait_for_result returns the payload"
    );

    // The parent aggregates both dependencies' results.
    let agg = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_dependency_results",
                &json!({ "thread_id": parent.id.0 }),
            )
            .await
            .unwrap(),
    );
    let deps = agg["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 2);
    let by_id = |tid: uuid::Uuid| {
        deps.iter()
            .find(|d| d["thread_id"] == json!(tid))
            .expect("dependency present")["result"]
            .clone()
    };
    assert_eq!(by_id(dep1.id.0), payload);
    assert_eq!(by_id(dep2.id.0), dep2_payload);
}

#[tokio::test]
async fn run_lineage_tools_home_fixture_and_attribute_nested() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};

    const PRODUCER_RUN_ID: &str = "aa4dc966-0e09-44c3-b7a5-2d048b48b301";
    const FIXTURE: &str =
        include_str!("../../../../maidan-types/tests/fixtures/waiter_result_v1.json");

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
    let ws = store
        .create_workspace(NewWorkspace {
            name: "lineage".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let parent = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parent".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let child = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: Some(parent.id),
            title: Some("nested".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        agent.id,
        ws.id,
        vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
    );
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let miss = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_thread_lineage",
                &json!({ "thread_id": parent.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert!(miss.is_null());

    let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture");
    assert_eq!(fixture["run_id"], json!(PRODUCER_RUN_ID));
    let lease = server
        .store
        .claim_lease_for(parent.id, agent.id)
        .await
        .unwrap();
    server
        .call_tool(
            &auth,
            "set_thread_result",
            &json!({ "thread_id": parent.id.0, "result": fixture, "claim_lease_id": lease.0 }),
        )
        .await
        .unwrap();
    // The write needed the claim; occupancy below counts the parent as queued.
    server.store.unassign_thread(parent.id).await.unwrap();

    let lined = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_thread_lineage",
                &json!({ "thread_id": parent.id.0 }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(lined["parent_run_id"], json!(PRODUCER_RUN_ID));
    assert_eq!(lined["thread_id"], json!(parent.id.0));

    let nest = unwrap_content(
        server
            .call_tool(
                &auth,
                "set_thread_lineage",
                &json!({ "thread_id": child.id.0, "parent_run_id": PRODUCER_RUN_ID }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(nest["parent_run_id"], json!(PRODUCER_RUN_ID));

    let listed = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_run_threads",
                &json!({ "parent_run_id": PRODUCER_RUN_ID }),
            )
            .await
            .unwrap(),
    );
    let ids: Vec<String> = listed
        .as_array()
        .expect("array")
        .iter()
        .map(|t| t["id"].as_str().expect("id").to_string())
        .collect();
    assert_eq!(
        ids,
        vec![parent.id.0.to_string(), child.id.0.to_string()],
        "nested child is attributed to the producer run"
    );

    let occ = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_run_occupancy",
                &json!({ "parent_run_id": PRODUCER_RUN_ID }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(occ["parent_run_id"], json!(PRODUCER_RUN_ID));
    assert_eq!(occ["open"], json!(2));
    // The parent holds its result, so claim_next will not hand it out.
    assert_eq!(occ["queued"], json!(1));
    assert_eq!(occ["blocked"], json!(1));

    store.mute_thread(agent.id, child.id).await.unwrap();
    let occ_muted = unwrap_content(
        server
            .call_tool(
                &auth,
                "get_run_occupancy",
                &json!({ "parent_run_id": PRODUCER_RUN_ID }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        occ_muted["open"],
        json!(2),
        "F7 mute is orthogonal — muted nested work still counts"
    );

    let blank = server
        .call_tool(
            &auth,
            "set_thread_lineage",
            &json!({ "thread_id": child.id.0, "parent_run_id": "   " }),
        )
        .await;
    assert!(
        matches!(&blank, Err(McpError::InvalidParams(m)) if m.contains("parent_run_id")),
        "blank producer id is rejected, not minted: {blank:?}"
    );
}

#[tokio::test]
async fn list_thread_results_filters_by_namespaced_kind() {
    use maidan_auth::capability::WORKSPACE_READ;

    const REVIEW: &str = "example.review.result/1";
    const PLAN: &str = "example.plan.result/1";

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
    let ws = store
        .create_workspace(NewWorkspace { name: "rk".into() })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let public = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "pub".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let private = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "priv".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let review = store
        .create_thread(NewThread {
            channel_id: public.id,
            parent_thread_id: None,
            title: Some("review".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .set_thread_result_claiming(review.id, agent.id, &json!({ "result_kind": REVIEW }))
        .await
        .unwrap();
    let plan = store
        .create_thread(NewThread {
            channel_id: public.id,
            parent_thread_id: None,
            title: Some("plan".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .set_thread_result_claiming(plan.id, agent.id, &json!({ "result_kind": PLAN }))
        .await
        .unwrap();
    let hidden = store
        .create_thread(NewThread {
            channel_id: private.id,
            parent_thread_id: None,
            title: Some("secret".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .set_thread_result_claiming(hidden.id, agent.id, &json!({ "result_kind": REVIEW }))
        .await
        .unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    // Session auth so can_access_thread hides the private-channel row.
    // produced_by is a real member (store-seeded; the tool itself is a read).
    let auth = AuthContext::from_session(agent.id, ws.id, vec![WORKSPACE_READ.to_string()]);
    let unwrap_content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };

    let all = unwrap_content(
        server
            .call_tool(&auth, "list_thread_results", &json!({}))
            .await
            .unwrap(),
    );
    let ids: Vec<String> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["thread_id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&review.id.0.to_string()));
    assert!(ids.contains(&plan.id.0.to_string()));
    assert!(
        !ids.contains(&hidden.id.0.to_string()),
        "a private-channel result the caller cannot access must not leak"
    );
    assert_eq!(ids.len(), 2);

    let reviews = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_thread_results",
                &json!({ "result_kind": REVIEW }),
            )
            .await
            .unwrap(),
    );
    let reviews = reviews.as_array().unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["thread_id"], json!(review.id.0));
    assert_eq!(reviews[0]["result"]["result_kind"], REVIEW);

    let none = unwrap_content(
        server
            .call_tool(
                &auth,
                "list_thread_results",
                &json!({ "result_kind": "decision" }),
            )
            .await
            .unwrap(),
    );
    assert!(
        none.as_array().unwrap().is_empty(),
        "the old closed-enum word is not a namespaced kind"
    );
}

#[tokio::test]
async fn set_thread_result_refuses_a_waiter_envelope_with_no_result_kind_and_stores_nothing() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_types::{NewThread, WAITER_RESULT_SCHEMA};

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
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let worker = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, worker.id).await.unwrap();
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        worker.id,
        ws.id,
        vec![THREAD_TRANSITION.to_string(), WORKSPACE_READ.to_string()],
    );
    let lease = store.claim_lease_for(thread.id, worker.id).await.unwrap();
    let args = |result: Value| json!({ "thread_id": thread.id.0, "claim_lease_id": lease.0, "result": result });

    let err = server
        .call_tool(
            &auth,
            "set_thread_result",
            &args(
                json!({ "schema": WAITER_RESULT_SCHEMA, "status": "reviewed",
                          "deliver_to": [{ "surface": "github", "repo": "o/r", "pr": 1 }] }),
            ),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, McpError::InvalidParams(m) if m.contains("`result_kind`")),
        "{err:?}"
    );
    assert!(
        store.get_thread_result(thread.id).await.unwrap().is_none(),
        "nothing stored"
    );

    server
        .call_tool(
            &auth,
            "set_thread_result",
            &args(
                json!({ "schema": WAITER_RESULT_SCHEMA, "result_kind": "k", "status": "reviewed" }),
            ),
        )
        .await
        .unwrap();
    assert!(store.get_thread_result(thread.id).await.unwrap().is_some());
}

#[tokio::test]
async fn set_thread_result_refuses_a_workflow_push_only_while_the_switch_is_on() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_types::{NewThread, CHANGE_RESULT_KIND, WAITER_RESULT_SCHEMA};

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
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let builder = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "builder".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, builder.id).await.unwrap();
    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let auth = AuthContext::from_session(
        builder.id,
        ws.id,
        vec![THREAD_TRANSITION.to_string(), WORKSPACE_READ.to_string()],
    );
    let lease = store.claim_lease_for(thread.id, builder.id).await.unwrap();
    let change = |diff: &str| {
        json!({
            "thread_id": thread.id.0,
            "claim_lease_id": lease.0,
            "result": {
                "schema": WAITER_RESULT_SCHEMA,
                "result_kind": CHANGE_RESULT_KIND,
                "status": "changed",
                "branch": "feature/agent-fix-ci-1a2b",
                "diff": diff,
                "deliver_to": [{"surface": "github_branch", "repo": "acme/app",
                                "branch": "feature/agent-fix-ci-1a2b", "base": "dev"}],
            },
        })
    };
    let workflow =
        "--- a/.github/workflows/test.yml\n+++ b/.github/workflows/test.yml\n@@ -1 +1 @@\n-a\n+b\n";
    let source = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n";

    server.set_refuse_workflow_paths(true);
    let err = server
        .call_tool(&auth, "set_thread_result", &change(workflow))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, McpError::InvalidParams(m)
            if m.contains("`.github/workflows/test.yml`") && m.contains("Workflows permission")),
        "{err:?}"
    );
    assert!(
        store.get_thread_result(thread.id).await.unwrap().is_none(),
        "nothing stored"
    );
    server
        .call_tool(&auth, "set_thread_result", &change(source))
        .await
        .unwrap();

    server.set_refuse_workflow_paths(false);
    server
        .call_tool(&auth, "set_thread_result", &change(workflow))
        .await
        .unwrap();
    let stored = store.get_thread_result(thread.id).await.unwrap().unwrap();
    assert_eq!(stored.result["diff"], json!(workflow));
}

#[tokio::test]
async fn critical_result_tool_blocks_close_until_a_human_approves() {
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_types::{NewThread, EXAMPLE_REVIEW_RESULT_KIND, REVIEW_SKILL, WAITER_RESULT_SCHEMA};

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
    let ws = store
        .create_workspace(NewWorkspace { name: "rv".into() })
        .await
        .unwrap();
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let reviewer = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "verifier".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let human = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "human".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .set_thread_owner(thread.id, Some(owner.id))
        .await
        .unwrap();
    // The review-skilled producer holds the thread: a result lands only under
    // the writer's live claim.
    store.assign_thread(thread.id, reviewer.id).await.unwrap();
    store
        .add_member_skill(reviewer.id, REVIEW_SKILL)
        .await
        .unwrap();

    let server = McpServer::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    let caps = vec![THREAD_TRANSITION.to_string(), WORKSPACE_READ.to_string()];
    let owner_auth = AuthContext::from_session(owner.id, ws.id, caps.clone());
    let rev = AuthContext::from_session(reviewer.id, ws.id, caps.clone());
    let human_auth = AuthContext::from_session(human.id, ws.id, caps);
    let content = |v: Value| -> Value {
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let tid = json!(thread.id.0);
    let lease = server
        .store
        .claim_lease_for(thread.id, reviewer.id)
        .await
        .unwrap();

    server
        .call_tool(
            &rev,
            "set_thread_result",
            &json!({
                "thread_id": tid,
                "claim_lease_id": lease.0,
                "result": {
                    "schema": WAITER_RESULT_SCHEMA,
                    "result_kind": EXAMPLE_REVIEW_RESULT_KIND,
                    "status": "reviewed",
                    "findings": [{ "severity": "critical" }],
                }
            }),
        )
        .await
        .unwrap();
    let status = content(
        server
            .call_tool(&rev, "get_review_status", &json!({ "thread_id": tid }))
            .await
            .unwrap(),
    );
    assert_eq!(status["required_count"], 1);
    assert_eq!(status["approvals_met"], json!(false));

    server
        .call_tool(
            &owner_auth,
            "transition_thread",
            &json!({
                "thread_id": tid,
                "action": "start_review",
            }),
        )
        .await
        .unwrap();
    let blocked = server
        .call_tool(
            &owner_auth,
            "transition_thread",
            &json!({
                "thread_id": tid,
                "action": "close",
            }),
        )
        .await;
    assert!(
        matches!(blocked, Err(McpError::InvalidParams(ref m)) if m.contains("review requirement")),
        "MCP-set critical result must block close, got {blocked:?}"
    );

    let root = handed_root(&store, thread.id).await;
    server
        .call_tool(
            &human_auth,
            "submit_review",
            &json!({ "thread_id": tid, "decision": "approve", "evidence_root": root }),
        )
        .await
        .unwrap();
    let closed = content(
        server
            .call_tool(
                &owner_auth,
                "transition_thread",
                &json!({
                    "thread_id": tid,
                    "action": "close",
                }),
            )
            .await
            .expect("human approve unblocks"),
    );
    assert_eq!(closed["state"], "closed");
}
