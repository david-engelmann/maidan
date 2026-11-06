//! A thread sent back three times waits on a person. A worker cannot decide
//! that gate or raise its cap. The person who sent it back can.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMaidanSession, NewMember, NewThread,
    NewWorkspace, WorkspaceId,
};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

const SESSION_SECRET: &[u8] = b"review-round-cap-session-secret-32";

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
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
    state.subscribe_resume_secret = Some(Arc::from(&b"review-round-cap-gate-secret-0001!"[..]));
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

async fn call(
    client: &reqwest::Client,
    method: Method,
    url: String,
    auth: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = client.request(method, url).header("authorization", auth);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let res = req.send().await.unwrap();
    let status = res.status();
    let body = res.json().await.unwrap_or(Value::Null);
    (status, body)
}

async fn hand_off(
    client: &reqwest::Client,
    base: &str,
    worker: &str,
    channel: uuid::Uuid,
    thread: uuid::Uuid,
) {
    let (status, claim) = call(
        client,
        Method::POST,
        format!("{base}/channels/{channel}/threads/claim-next"),
        worker,
        Some(json!({"lease_secs": 60})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    assert_eq!(claim["id"], json!(thread));
    let lease = claim["claim_lease_id"].clone();
    let (status, body) = call(
        client,
        Method::PUT,
        format!("{base}/threads/{thread}/result"),
        worker,
        Some(json!({"result": {"status": "done"}, "claim_lease_id": lease})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        client,
        Method::POST,
        format!("{base}/threads/{thread}"),
        worker,
        Some(json!({"action": "start_review"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        client,
        Method::POST,
        format!("{base}/threads/{thread}/claim/release"),
        worker,
        Some(json!({"claim_lease_id": lease})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn mcp(client: &reqwest::Client, base: &str, token: &str, name: &str, args: Value) -> Value {
    client
        .post(format!("{base}/mcp"))
        .header("authorization", bearer(token))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn refusal(res: &Value) -> String {
    if let Some(error) = res.get("error") {
        return error["message"].as_str().unwrap_or_default().to_string();
    }
    res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

async fn signed_in(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let session = store
        .create_session(NewMaidanSession {
            workspace_id: ws,
            member_id: member,
            api_token_id: None,
            oidc_identity_id: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    maidan_server::session::set_session_cookie(
        &mut headers,
        session.id,
        3600,
        false,
        SESSION_SECRET,
    )
    .unwrap();
    headers
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn three_send_backs_wait_on_a_person_and_a_worker_cannot_wave_them_through() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace {
            name: "rounds".into(),
        })
        .await
        .unwrap();
    let worker_m = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let reviewer_m = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "reviewer".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let admin_m = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let work = vec![
        capability::WORKSPACE_READ.into(),
        capability::WORKSPACE_WRITE.into(),
        capability::THREAD_TRANSITION.into(),
    ];
    let worker = mint(store.as_ref(), ws.id, worker_m.id, work.clone()).await;
    let reviewer = mint(store.as_ref(), ws.id, reviewer_m.id, work).await;
    let admin = mint(
        store.as_ref(),
        ws.id,
        admin_m.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
            capability::THREAD_TRANSITION.into(),
            capability::CHANNEL_ADMIN.into(),
        ],
    )
    .await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("fix the gate".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let tid = thread.id.0;
    let worker_h = bearer(&worker);
    let reviewer_h = bearer(&reviewer);

    for n in 1..=3 {
        hand_off(&client, &base, &worker_h, channel.id.0, tid).await;
        let (status, body) = call(
            &client,
            Method::POST,
            format!("{base}/threads/{tid}/reviews"),
            &reviewer_h,
            Some(json!({"decision": "request_changes", "note": format!("round {n}")})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "send-back {n}: {body}");
    }

    let (status, claim) = call(
        &client,
        Method::POST,
        format!("{base}/channels/{}/threads/claim-next", channel.id.0),
        &worker_h,
        Some(json!({"lease_secs": 60})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    assert!(
        claim.is_null(),
        "the third send-back is not claimable: {claim}"
    );

    let (status, inbox) = call(
        &client,
        Method::GET,
        format!("{base}/members/{}/waiting", reviewer_m.id.0),
        &reviewer_h,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{inbox}");
    let item = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "open_gate")
        .expect("the inbox names the gate");
    assert_eq!(item["thread_id"], json!(tid));
    assert!(
        item["summary"]
            .as_str()
            .unwrap()
            .starts_with("Sent back 3 times in review"),
        "{}",
        item["summary"]
    );

    let waiting = mcp(
        &client,
        &base,
        &reviewer,
        "get_waiting_inbox",
        json!({"member_id": reviewer_m.id.0}),
    )
    .await;
    let text = waiting.to_string();
    assert!(
        text.contains("Sent back 3 times in review") && text.contains("open_gate"),
        "{text}"
    );

    let (status, list) = call(
        &client,
        Method::GET,
        format!("{base}/workspaces/{}/approval-gates", ws.id.0),
        &worker_h,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let view = list
        .as_array()
        .unwrap()
        .iter()
        .find(|g| {
            g["gate"]["thread_id"] == json!(tid.to_string()) || g["gate"]["thread_id"] == json!(tid)
        })
        .expect("listed");
    let gate_id = view["gate"]["id"].as_str().unwrap().to_string();
    let request_state = view["request_state"].as_str().unwrap().to_string();
    assert!(view["gate"]["prompt"]
        .as_str()
        .unwrap()
        .contains("latest note: round 3"));

    for action in ["decline", "cancel", "accept"] {
        let (status, body) = call(
            &client,
            Method::POST,
            format!("{base}/approval-gates/{gate_id}/answer"),
            &worker_h,
            Some(json!({"request_state": request_state, "action": action})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{action}: {body}");
        assert!(
            body["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("only a person or channel admin can decide a review-round cap gate"),
            "{action}: {body}"
        );
    }
    for decision in ["decline", "accept"] {
        let res = mcp(
            &client,
            &base,
            &worker,
            "approval_decide",
            json!({"gate_id": gate_id, "decision": decision}),
        )
        .await;
        let said = refusal(&res);
        assert!(
            said.contains("only a person or channel admin can decide a review-round cap gate"),
            "{decision}: {res}"
        );
    }

    let path = format!("/threads/{tid}/review-requirement");
    for rounds in [0, 5] {
        let (status, body) = call(
            &client,
            Method::PUT,
            format!("{base}{path}"),
            &worker_h,
            Some(json!({"required_count": 0, "max_rounds": rounds})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "max_rounds {rounds}: {body}");
        let res = mcp(
            &client,
            &base,
            &worker,
            "set_review_requirement",
            json!({"thread_id": tid, "required_count": 0, "max_rounds": rounds}),
        )
        .await;
        assert!(
            refusal(&res).contains("channel:admin"),
            "mcp max_rounds {rounds}: {res}"
        );
    }
    let (status, body) = call(
        &client,
        Method::PUT,
        format!("{base}{path}"),
        &worker_h,
        Some(json!({"required_count": 0, "max_rounds": 1})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "tightening stays a transition: {body}"
    );
    assert_eq!(body["max_rounds"], 1);

    let (status, body) = call(
        &client,
        Method::PUT,
        format!("{base}{path}"),
        &bearer(&admin),
        Some(json!({"required_count": 0, "max_rounds": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        &client,
        Method::PUT,
        format!("{base}{path}"),
        &bearer(&admin),
        Some(json!({"required_count": 0, "max_rounds": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["max_rounds"], 0);
    let audit = store.list_audit_for_workspace(ws.id, 50).await.unwrap();
    assert!(
        audit.iter().any(|row| {
            row.action == "review_requirement.lower"
                && row.metadata["previous_max_rounds"] == 5
                && row.metadata["max_rounds"] == 0
        }),
        "{audit:?}"
    );

    // Put the cap back so the open gate is still the one the person decides.
    // The gate was opened at the default of 3 and is still pending.
    let cookie = signed_in(store.as_ref(), ws.id, reviewer_m.id).await;
    let listed = client
        .get(format!(
            "{base}/ui/api/workspaces/{}/approval-gates",
            ws.id.0
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .header("sec-fetch-site", "same-origin")
        .header(reqwest::header::ORIGIN, &base)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed: Vec<Value> = listed.json().await.unwrap();
    let state = listed
        .iter()
        .find(|g| g["gate"]["id"] == json!(gate_id))
        .unwrap()["request_state"]
        .as_str()
        .unwrap()
        .to_string();
    let accepted = client
        .post(format!("{base}/ui/api/approval-gates/{gate_id}/answer"))
        .header(reqwest::header::COOKIE, &cookie)
        .header("sec-fetch-site", "same-origin")
        .header(reqwest::header::ORIGIN, &base)
        .json(&json!({"request_state": state, "action": "accept"}))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let (status, claim) = call(
        &client,
        Method::POST,
        format!("{base}/channels/{}/threads/claim-next", channel.id.0),
        &worker_h,
        Some(json!({"lease_secs": 60})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    assert_eq!(claim["id"], json!(tid));

    // A decline on a second thread keeps it off the queue. The reviewer is
    // again the person who sent it back. Its own channel, so claim cannot
    // hand back the thread that was just accepted.
    let other_channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "other".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let other = store
        .create_thread(NewThread {
            channel_id: other_channel.id,
            parent_thread_id: None,
            title: Some("the other task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    // Cap of 1, which a transitioner may set.
    let (status, body) = call(
        &client,
        Method::PUT,
        format!("{base}/threads/{}/review-requirement", other.id.0),
        &reviewer_h,
        Some(json!({"required_count": 0, "max_rounds": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    hand_off(&client, &base, &worker_h, other_channel.id.0, other.id.0).await;
    let (status, body) = call(
        &client,
        Method::POST,
        format!("{base}/threads/{}/reviews", other.id.0),
        &reviewer_h,
        Some(json!({"decision": "request_changes", "note": "enough"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, list) = call(
        &client,
        Method::GET,
        format!("{base}/workspaces/{}/approval-gates", ws.id.0),
        &reviewer_h,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let view = list
        .as_array()
        .unwrap()
        .iter()
        .find(|g| {
            g["gate"]["thread_id"] == json!(other.id.0)
                || g["gate"]["thread_id"] == json!(other.id.0.to_string())
        })
        .expect("the second gate");
    let other_gate = view["gate"]["id"].as_str().unwrap();
    let other_state = view["request_state"].as_str().unwrap();
    let cookie = signed_in(store.as_ref(), ws.id, reviewer_m.id).await;
    let declined = client
        .post(format!("{base}/ui/api/approval-gates/{other_gate}/answer"))
        .header(reqwest::header::COOKIE, &cookie)
        .header("sec-fetch-site", "same-origin")
        .header(reqwest::header::ORIGIN, &base)
        .json(&json!({"request_state": other_state, "action": "decline"}))
        .send()
        .await
        .unwrap();
    assert_eq!(declined.status(), StatusCode::OK);
    let (status, again) = call(
        &client,
        Method::POST,
        format!("{base}/channels/{}/threads/claim-next", other_channel.id.0),
        &worker_h,
        Some(json!({"lease_secs": 60})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert!(
        again.is_null(),
        "a declined round stays off the queue: {again}"
    );

    // The confirmation link names the reviewer who opened the gate. Spending
    // it from that person's signed-in session accepts the round.
    let confirm_channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "confirm".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let confirm_thread = store
        .create_thread(NewThread {
            channel_id: confirm_channel.id,
            parent_thread_id: None,
            title: Some("confirm the round".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let (status, body) = call(
        &client,
        Method::PUT,
        format!("{base}/threads/{}/review-requirement", confirm_thread.id.0),
        &reviewer_h,
        Some(json!({"required_count": 0, "max_rounds": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    hand_off(
        &client,
        &base,
        &worker_h,
        confirm_channel.id.0,
        confirm_thread.id.0,
    )
    .await;
    let (status, body) = call(
        &client,
        Method::POST,
        format!("{base}/threads/{}/reviews", confirm_thread.id.0),
        &reviewer_h,
        Some(json!({"decision": "request_changes", "note": "confirm me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, list) = call(
        &client,
        Method::GET,
        format!("{base}/workspaces/{}/approval-gates", ws.id.0),
        &reviewer_h,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let view = list
        .as_array()
        .unwrap()
        .iter()
        .find(|g| {
            g["gate"]["thread_id"] == json!(confirm_thread.id.0)
                || g["gate"]["thread_id"] == json!(confirm_thread.id.0.to_string())
        })
        .expect("the confirmation gate");
    let confirm_gate = view["gate"]["id"].as_str().unwrap();
    let asked = mcp(
        &client,
        &base,
        &reviewer,
        "approval_decide",
        json!({"gate_id": confirm_gate, "decision": "accept"}),
    )
    .await;
    let text = asked["result"]["content"][0]["text"].as_str().unwrap();
    let payload: Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["status"], "confirmation_required", "{payload}");
    let url = payload["confirmation_url"].as_str().unwrap();
    let fragment = url.split("#confirm-approval=").nth(1).unwrap();
    let (gate_part, token) = fragment.split_once('.').unwrap();
    assert_eq!(gate_part, confirm_gate);
    let confirmed = client
        .post(format!("{base}/auth/approval-confirmations/confirm"))
        .header(reqwest::header::COOKIE, &cookie)
        .header("sec-fetch-site", "same-origin")
        .header(reqwest::header::ORIGIN, &base)
        .json(&json!({"gate_id": confirm_gate, "token": token}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        confirmed.status(),
        StatusCode::OK,
        "{}",
        confirmed.text().await.unwrap()
    );
    let (status, claimed) = call(
        &client,
        Method::POST,
        format!(
            "{base}/channels/{}/threads/claim-next",
            confirm_channel.id.0
        ),
        &worker_h,
        Some(json!({"lease_secs": 60})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    assert_eq!(claimed["id"], json!(confirm_thread.id.0));
}
