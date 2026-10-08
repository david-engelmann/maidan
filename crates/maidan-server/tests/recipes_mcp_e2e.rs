//! The compose recipes' MCP calls against a real server, with auth on.
//!
//! `examples/recipes/maidan_http.py` is the recipes' own standard-library
//! client. Its `tool()` speaks MCP `2026-07-28`, and the server holds every
//! request on that revision to its rules: the version header, the same version
//! in `params._meta` beside the client's capabilities, and the `Mcp-Method`
//! and `Mcp-Name` routing headers. The helper once sent only the header, so
//! every recipe tool call was refused with
//! `params._meta must carry io.modelcontextprotocol/protocolVersion` and
//! nothing caught it: the recipes run in compose, which CI does not start.
//!
//! This runs the deploy recipe's agent (`deploy_agent.py --once`) unmodified
//! apart from where it reads its credentials. It claims the task, opens a gate
//! with `request_approval`, and polls `get_approval_gate` until a person
//! answers. The test declines the gate as that person, so nothing is accepted,
//! and the agent records that it did not deploy. Needs `python3`.

use std::{
    io::Read,
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{atomic::AtomicI64, Arc},
    time::{Duration, Instant},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, Arc<dyn Store>, tempfile::TempDir) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("PRAGMA busy_timeout = 5000")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path().join("artifacts"))),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // Listing gates signs each `request_state` with the server secret.
    state.subscribe_resume_secret = Some(Arc::from(&b"recipes-mcp-e2e-secret-key-00001!"[..]));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, store, dir)
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str, kind: MemberKind) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind,
        })
        .await
        .unwrap()
        .id
}

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: &[&str]) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

/// Runs `deploy_agent.main()` from the recipes directory with its credentials
/// read from `creds` instead of `/creds/agent.json`, the compose mount.
const HARNESS: &str = r#"
import pathlib, sys
recipes, creds = pathlib.Path(sys.argv[1]), sys.argv[2]
sys.path.insert(0, str(recipes))
import maidan_http, deploy_agent
deploy_agent.from_creds = lambda: maidan_http.from_creds(creds)
sys.argv = ["deploy_agent.py", "--once"]
raise SystemExit(deploy_agent.main())
"#;

#[tokio::test]
async fn the_deploy_recipe_opens_and_reads_its_gate_over_mcp_2026_07_28() {
    let (addr, store, dir) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "recipe".into(),
        })
        .await
        .unwrap();
    let agent = member(store.as_ref(), ws.id, "deployer", MemberKind::Agent).await;
    let person = member(store.as_ref(), ws.id, "oncall", MemberKind::Human).await;
    // The deployer's scope as `provision.py` mints it.
    let agent_token = mint(
        store.as_ref(),
        ws.id,
        agent,
        &[
            capability::WORKSPACE_READ,
            capability::MESSAGE_POST,
            capability::THREAD_TRANSITION,
        ],
    )
    .await;
    let person_token = mint(
        store.as_ref(),
        ws.id,
        person,
        &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE],
    )
    .await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "deploys".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("Deploy v9 to production".into()),
            description: None,
        })
        .await
        .unwrap();

    let creds = dir.path().join("agent.json");
    std::fs::write(
        &creds,
        json!({
            "url": base,
            "token": agent_token,
            "channel_id": channel.id.0.to_string(),
        })
        .to_string(),
    )
    .unwrap();
    let recipes = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/recipes");
    let mut child = Command::new("python3")
        .arg("-B")
        .arg("-c")
        .arg(HARNESS)
        .arg(&recipes)
        .arg(&creds)
        .env("MAIDAN_URL", &base)
        .env("POLL_SECS", "0.2")
        .env_remove("DEPLOY_COMMAND")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("python3 is required to run the recipes' MCP calls: {err}"));

    // The agent's `request_approval` opens a gate on the thread it claimed.
    // Answer it as the person: decline, so this test accepts nothing.
    let http = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    let (gate_id, request_state) = loop {
        if let Some(status) = child.try_wait().unwrap() {
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("the deploy agent exited ({status}) before opening a gate:\n{stderr}");
        }
        assert!(
            Instant::now() < deadline,
            "the deploy agent opened no gate in 60s"
        );
        let views: Vec<Value> = http
            .get(format!("{base}/workspaces/{}/approval-gates", ws.id.0))
            .bearer_auth(&person_token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let found = views.iter().find_map(|view| {
            let gate = view.get("gate").unwrap_or(view);
            (gate["thread_id"] == json!(thread.id.0.to_string())).then(|| {
                (
                    gate["id"].as_str().unwrap().to_string(),
                    view["request_state"].as_str().unwrap().to_string(),
                )
            })
        });
        if let Some(found) = found {
            break found;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let declined = http
        .post(format!("{base}/approval-gates/{gate_id}/answer"))
        .bearer_auth(&person_token)
        .json(&json!({"action": "decline", "request_state": request_state}))
        .send()
        .await
        .unwrap();
    assert_eq!(declined.status(), StatusCode::OK);

    // `get_approval_gate` sees the decline, and the agent stops without deploying.
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "deploy_agent.py failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(&format!("waiting on gate {gate_id}")),
        "{stdout}"
    );
    assert!(
        stdout.contains("gate declined, not deploying"),
        "{stdout}"
    );
    let result: Value = http
        .get(format!("{base}/threads/{}/result", thread.id.0))
        .bearer_auth(&person_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let result = result.get("result").unwrap_or(&result);
    assert_eq!(
        result,
        &json!({"status": "not_deployed", "gate": "declined"}),
        "the agent records that it did not deploy"
    );
}
