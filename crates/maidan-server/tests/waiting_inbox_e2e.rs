//! The waiting-on-you inbox over REST: the route composes a member's assigned
//! non-terminal threads + the reviews requested from them + the workspace's
//! pending approval gates (mentions are
//! covered by the pure `assemble_waiting_inbox` unit test). Auth-enabled +
//! self-only, with a minted token that IS the acting member.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_fsm::ThreadAction;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewApprovalGate, NewChannel, NewMember, NewThread, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn waiting_inbox_composes_assigned_threads_review_requests_and_open_gates() {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    // An assigned, non-terminal thread.
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some("do the thing".into()),
            description: None,
        })
        .await
        .unwrap();
    store.assign_thread(thread.id, member.id).await.unwrap();
    // A pending approval gate.
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: member.id,
            prompt: "approve the deploy".into(),
            schema: None,
        })
        .await
        .unwrap();

    // A task an agent handed to review, naming the member as its reviewer.
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "coder".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let review_ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "review".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let reviewed = store
        .create_thread(NewThread {
            channel_id: review_ch.id,
            parent_thread_id: None,
            title: Some("fix the flaky test".into()),
            description: None,
        })
        .await
        .unwrap();
    store
        .claim_next_thread(review_ch.id, agent.id, Some(60))
        .await
        .unwrap()
        .expect("claimable");
    store
        .transition_thread(reviewed.id, agent.id, ThreadAction::StartReview)
        .await
        .unwrap();
    store.add_reviewer(reviewed.id, member.id).await.unwrap();

    // A review in a private channel the member is not in. Naming them as a
    // reviewer does not let them open it, so its title must not reach them.
    let secret_ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    let hidden = store
        .create_thread(NewThread {
            channel_id: secret_ch.id,
            parent_thread_id: None,
            title: Some("private acquisition plan".into()),
            description: None,
        })
        .await
        .unwrap();
    // The agent works in the private channel; the reviewer does not.
    store
        .add_channel_member(
            secret_ch.id,
            agent.id,
            maidan_types::ChannelMemberRole::Member,
        )
        .await
        .unwrap();
    store
        .claim_next_thread(secret_ch.id, agent.id, Some(60))
        .await
        .unwrap()
        .expect("claimable");
    store
        .transition_thread(hidden.id, agent.id, ThreadAction::StartReview)
        .await
        .unwrap();
    store.add_reviewer(hidden.id, member.id).await.unwrap();

    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("w".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let auth = format!("Bearer {}", secret.as_str());

    let inbox: serde_json::Value = client
        .get(format!("{base}/members/{}/waiting", member.id.0))
        .header("Authorization", &auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        inbox["total"], 3,
        "one assigned thread + one requested review + one open gate"
    );
    let kinds: Vec<&str> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"assigned_thread"));
    assert!(kinds.contains(&"open_gate"));
    let review = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "review_request")
        .expect("the requested review is waiting on the member");
    assert_eq!(review["thread_id"], serde_json::json!(reviewed.id.0));
    assert_eq!(review["summary"], "fix the flaky test");
    assert_eq!(inbox["sla_secs"], 86400);
    assert!(
        !inbox.to_string().contains("private acquisition plan"),
        "a review in a private channel the member cannot open is not listed"
    );

    // The MCP tool applies the same rule.
    let mcp: serde_json::Value = client
        .post(format!("{base}/mcp"))
        .header("Authorization", &auth)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "get_waiting_inbox", "arguments": {"member_id": member.id.0}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = mcp.to_string();
    assert!(
        text.contains("fix the flaky test"),
        "MCP lists the open review: {text}"
    );
    assert!(
        !text.contains("private acquisition plan"),
        "MCP hides the private review too"
    );
    // Freshly-created items are not yet overdue (the overdue math is unit-tested
    // against aged items in `assemble_waiting_inbox`).
    assert_eq!(inbox["overdue"], 0);

    server.abort();
}

/// A server on in-memory SQLite with auth on, for the tests below.
async fn spawn() -> (
    Arc<dyn Store>,
    String,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
) {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.keep()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    (store, format!("http://{addr}"), client, server)
}

async fn member_with_token(
    store: &Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    handle: &str,
    caps: &[&str],
) -> (maidan_types::MemberId, String) {
    member_of_kind_with_token(store, ws, handle, caps, MemberKind::Human).await
}

async fn member_of_kind_with_token(
    store: &Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    handle: &str,
    caps: &[&str],
    kind: MemberKind,
) -> (maidan_types::MemberId, String) {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some(handle.into()),
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    (member.id, format!("Bearer {}", secret.as_str()))
}

/// A thread an agent worked and handed to review with no reviewer named.
async fn in_review_unnamed(
    store: &Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    agent: maidan_types::MemberId,
    title: &str,
    private: bool,
) -> maidan_types::ThreadId {
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("ch-{}", uuid::Uuid::now_v7()),
            topic: None,
            private,
        })
        .await
        .unwrap();
    if private {
        store
            .add_channel_member(ch.id, agent, maidan_types::ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
        })
        .await
        .unwrap();
    store.claim_thread(thread.id, agent).await.unwrap();
    store
        .transition_thread(thread.id, agent, ThreadAction::StartReview)
        .await
        .unwrap();
    thread.id
}

async fn unassigned_titles(
    client: &reqwest::Client,
    base: &str,
    auth: &str,
    member: maidan_types::MemberId,
) -> Vec<String> {
    let inbox: serde_json::Value = client
        .get(format!("{base}/members/{}/waiting", member.0))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "unassigned_review")
        .map(|i| i["summary"].as_str().unwrap().to_string())
        .collect()
}

/// A review that names nobody reaches its owner, or, with no owner, a
/// workspace admin; never a member of another workspace, never someone who
/// cannot open the thread, and never a member who is neither.
#[tokio::test]
async fn a_review_nobody_was_named_for_reaches_its_owner_or_an_admin_and_no_one_else() {
    let (store, base, client, server) = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let other = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let (admin, admin_auth) = member_with_token(
        &store,
        ws,
        "admin",
        &["workspace:read", "thread:transition", "token:admin"],
    )
    .await;
    let (owner, owner_auth) = member_with_token(&store, ws, "owner", &["workspace:read"]).await;
    let (bystander, bystander_auth) =
        member_with_token(&store, ws, "bystander", &["workspace:read"]).await;
    let (agent, _) = member_with_token(&store, ws, "agent", &["workspace:read"]).await;
    let (other_admin, other_admin_auth) =
        member_with_token(&store, other, "admin", &["workspace:read", "token:admin"]).await;
    let (other_agent, _) = member_with_token(&store, other, "agent", &["workspace:read"]).await;

    let owned = in_review_unnamed(&store, ws, agent, "owned: the retry budget", false).await;
    store.set_thread_owner(owned, Some(owner)).await.unwrap();
    in_review_unnamed(&store, ws, agent, "ownerless: the cache header", false).await;
    in_review_unnamed(&store, ws, agent, "private: the acquisition", true).await;
    in_review_unnamed(&store, other, other_agent, "tenant b: the audit", false).await;

    assert_eq!(
        unassigned_titles(&client, &base, &owner_auth, owner).await,
        vec!["owned: the retry budget"],
        "the owner hears about its own review, and nothing it does not own"
    );
    assert_eq!(
        unassigned_titles(&client, &base, &admin_auth, admin).await,
        vec!["ownerless: the cache header"],
        "an admin hears the ownerless review it can open; not the owned one, \
         not the private one, not another workspace's"
    );
    assert!(
        unassigned_titles(&client, &base, &bystander_auth, bystander)
            .await
            .is_empty(),
        "a member who neither owns nor administers hears nothing"
    );
    assert_eq!(
        unassigned_titles(&client, &base, &other_admin_auth, other_admin).await,
        vec!["tenant b: the audit"],
        "the other workspace's admin hears its own, and nothing of this one"
    );
    let cross = client
        .get(format!("{base}/members/{}/waiting", admin.0))
        .header("Authorization", &other_admin_auth)
        .send()
        .await
        .unwrap();
    assert!(
        !cross.text().await.unwrap().contains("ownerless"),
        "another workspace's admin cannot read this workspace's queue"
    );

    // The MCP tool applies the same rule.
    let mcp: serde_json::Value = client
        .post(format!("{base}/mcp"))
        .header("Authorization", &admin_auth)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "get_waiting_inbox", "arguments": {"member_id": admin.0}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = mcp.to_string();
    assert!(
        text.contains("unassigned_review") && text.contains("ownerless: the cache header"),
        "{text}"
    );
    for hidden in [
        "owned: the retry budget",
        "private: the acquisition",
        "tenant b: the audit",
    ] {
        assert!(!text.contains(hidden), "MCP must not list {hidden}: {text}");
    }

    server.abort();
}

async fn questions(
    client: &reqwest::Client,
    base: &str,
    auth: &str,
    member: maidan_types::MemberId,
) -> Vec<String> {
    let inbox: serde_json::Value = client
        .get(format!("{base}/members/{}/waiting", member.0))
        .header("Authorization", auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut asked: Vec<String> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "question")
        .map(|i| i["summary"].as_str().unwrap().to_string())
        .collect();
    asked.sort();
    asked
}

/// A thread whose agent holds the claim and asks a question.
async fn asked(
    store: &Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    agent: maidan_types::MemberId,
    title: &str,
    private: bool,
) -> maidan_types::ThreadId {
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: format!("ch-{}", uuid::Uuid::now_v7()),
            topic: None,
            private,
        })
        .await
        .unwrap();
    if private {
        store
            .add_channel_member(ch.id, agent, maidan_types::ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    let thread = store
        .create_thread(NewThread {
            channel_id: ch.id,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
        })
        .await
        .unwrap();
    store.claim_thread(thread.id, agent).await.unwrap();
    store
        .declare_thread_status(
            thread.id,
            maidan_types::DeclaredStatus::NeedsInput,
            "which region?".into(),
            agent,
        )
        .await
        .unwrap();
    thread.id
}

/// An agent's question reaches the thread's owner, or a workspace admin when
/// nobody owns the thread or the owner is the one asking. It never reaches the
/// asker, a bystander, someone who cannot open the thread, or another
/// workspace, and a human's answer in the thread takes it off the queue.
#[tokio::test]
async fn an_agents_question_reaches_its_owner_or_an_admin_until_a_human_answers() {
    let (store, base, client, server) = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let other = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let (admin, admin_auth) =
        member_with_token(&store, ws, "admin", &["workspace:read", "token:admin"]).await;
    let (owner, owner_auth) =
        member_with_token(&store, ws, "owner", &["workspace:read", "message:post"]).await;
    let (bystander, bystander_auth) =
        member_with_token(&store, ws, "bystander", &["workspace:read"]).await;
    let (agent, agent_auth) =
        member_of_kind_with_token(&store, ws, "agent", &["workspace:read"], MemberKind::Agent)
            .await;
    let (other_admin, other_admin_auth) =
        member_with_token(&store, other, "admin", &["workspace:read", "token:admin"]).await;
    let (other_agent, _) = member_of_kind_with_token(
        &store,
        other,
        "agent",
        &["workspace:read"],
        MemberKind::Agent,
    )
    .await;

    let owned = asked(&store, ws, agent, "owned", false).await;
    store.set_thread_owner(owned, Some(owner)).await.unwrap();
    asked(&store, ws, agent, "ownerless", false).await;
    let self_owned = asked(&store, ws, agent, "asker owns it", false).await;
    store
        .set_thread_owner(self_owned, Some(agent))
        .await
        .unwrap();
    asked(&store, ws, agent, "private", true).await;
    asked(&store, other, other_agent, "tenant b", false).await;
    // A person may declare needs_input on a task they hold; it is not an
    // agent's question and reaches nobody.
    let by_person = asked(&store, ws, bystander, "a person asked", false).await;
    store
        .set_thread_owner(by_person, Some(owner))
        .await
        .unwrap();

    assert_eq!(
        questions(&client, &base, &owner_auth, owner).await,
        vec!["owned: which region?"],
        "the owner hears the question on its thread, and only that one"
    );
    assert_eq!(
        questions(&client, &base, &admin_auth, admin).await,
        vec!["asker owns it: which region?", "ownerless: which region?"],
        "an admin hears an ownerless question and one the owner asked itself; \
         not the owned one, not the private one, not another workspace's"
    );
    assert!(
        questions(&client, &base, &agent_auth, agent)
            .await
            .is_empty(),
        "the asker is never asked its own question"
    );
    assert!(
        questions(&client, &base, &bystander_auth, bystander)
            .await
            .is_empty(),
        "a member who neither owns nor administers hears nothing"
    );
    assert_eq!(
        questions(&client, &base, &other_admin_auth, other_admin).await,
        vec!["tenant b: which region?"],
        "the other workspace's admin hears its own, and nothing of this one"
    );

    // The MCP tool applies the same rule.
    let mcp: serde_json::Value = client
        .post(format!("{base}/mcp"))
        .header("Authorization", &owner_auth)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "get_waiting_inbox", "arguments": {"member_id": owner.0}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = mcp.to_string();
    assert!(
        text.contains(r#"\"question\""#) && text.contains("owned: which region?"),
        "{text}"
    );
    for hidden in ["ownerless", "asker owns it", "private", "tenant b"] {
        assert!(!text.contains(hidden), "MCP must not list {hidden}: {text}");
    }

    // The owner answers in the thread, and the question leaves the queue.
    let answered = client
        .post(format!("{base}/threads/{}/messages", owned.0))
        .header("Authorization", &owner_auth)
        .json(&serde_json::json!({"body": "us-east-1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(answered.status(), reqwest::StatusCode::CREATED);
    assert!(
        questions(&client, &base, &owner_auth, owner)
            .await
            .is_empty(),
        "an answered question waits on nobody"
    );

    server.abort();
}

/// On a thread whose close needs an approval, start_review is refused until a
/// result is posted, with the fix named. Without a requirement it is not.
#[tokio::test]
async fn start_review_on_a_gated_thread_needs_a_posted_result() {
    let (store, base, client, server) = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let (worker, auth) = member_with_token(
        &store,
        ws,
        "worker",
        &["workspace:read", "thread:transition"],
    )
    .await;
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = |title: &'static str| NewThread {
        channel_id: ch.id,
        parent_thread_id: None,
        title: Some(title.into()),
        description: None,
    };
    let gated = store.create_thread(thread("gated")).await.unwrap().id;
    store.set_review_requirement(gated, 1).await.unwrap();
    let start = |id: maidan_types::ThreadId| {
        client
            .post(format!("{base}/threads/{}", id.0))
            .header("Authorization", &auth)
            .json(&serde_json::json!({"action": "start_review"}))
            .send()
    };
    let refused = start(gated).await.unwrap();
    assert_eq!(refused.status(), 409);
    let problem: serde_json::Value = refused.json().await.unwrap();
    assert!(
        problem["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("no result posted"),
        "{problem}"
    );
    store
        .set_thread_result(gated, worker, &serde_json::json!({"status": "done"}))
        .await
        .unwrap();
    assert_eq!(start(gated).await.unwrap().status(), 200);

    let ungated = store.create_thread(thread("ungated")).await.unwrap().id;
    assert_eq!(start(ungated).await.unwrap().status(), 200);

    server.abort();
}
