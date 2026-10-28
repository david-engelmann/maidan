//! The approval card's two session-proxied thread reads, with auth on and two
//! workspaces: `GET /ui/api/threads/{tid}/reviews` (who decided, after a
//! reload) and `GET /ui/api/threads/{tid}/artifacts` (who linked each artifact
//! and when). They serve the bearer tree's handlers, so they answer exactly
//! when the packet read does: a signed-in member of the thread's workspace
//! reads them, another workspace's session gets nothing of them, and neither
//! does a member outside a private channel.

use maidan_store::test_support::SetResultClaiming;
use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelMemberRole, MemberId, MemberKind, NewChannel, NewMaidanSession, NewMember, NewThread,
    NewWorkspace, ReviewDecision, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const SESSION_SECRET: &[u8] = b"ui-thread-reads-e2e-session-secret!!";
const HELD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DROPPED: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    // Gate lists sign each gate's `request_state`; without the key they 500.
    state.subscribe_resume_secret = Some(Arc::from(
        maidan_server::subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET,
    ));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
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

/// A person signed in through the identity provider: a session with no token
/// behind it, as the console holds after an OIDC sign-in.
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

struct Tenant {
    workspace: WorkspaceId,
    channel: maidan_types::ChannelId,
    operator_id: MemberId,
    message: maidan_types::MessageId,
    thread: ThreadId,
    private_thread: ThreadId,
    worker: MemberId,
    reviewer: MemberId,
    /// A signed-in member of the workspace and of its private channel.
    operator: String,
    /// A signed-in member of the workspace but not of its private channel.
    outsider: String,
}

/// A workspace whose task was handed to review with one artifact linked and
/// one linked then unlinked, and approved by a reviewer against its packet.
/// The same hand-off sits in a private channel.
async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let worker = member(store, ws, "worker", MemberKind::Agent).await;
    let reviewer = member(store, ws, "rae", MemberKind::Human).await;
    let operator = member(store, ws, "operator", MemberKind::Human).await;
    let outsider = member(store, ws, "outsider", MemberKind::Human).await;
    for sha in [HELD, DROPPED] {
        store.record_artifact_ref(ws, sha).await.unwrap();
    }
    let mut threads = Vec::new();
    let mut channels = Vec::new();
    for (channel_name, private) in [("work", false), ("secret", true)] {
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws,
                name: channel_name.into(),
                topic: None,
                private,
            })
            .await
            .unwrap();
        if private {
            for m in [worker, reviewer, operator] {
                store
                    .add_channel_member(channel.id, m, ChannelMemberRole::Member)
                    .await
                    .unwrap();
            }
        }
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(distinct_brief("Ship it")),
                description: None,
                created_by: None,
            })
            .await
            .unwrap()
            .id;
        store.claim_thread(thread, worker).await.unwrap();
        store
            .set_thread_result_claiming(thread, worker, &json!({"status": "done"}))
            .await
            .unwrap();
        store
            .link_thread_artifact(thread, HELD, worker)
            .await
            .unwrap();
        store
            .link_thread_artifact(thread, DROPPED, worker)
            .await
            .unwrap();
        store
            .transition_thread(thread, worker, maidan_fsm::ThreadAction::StartReview)
            .await
            .unwrap();
        let packet = store.latest_review_packet(thread).await.unwrap().unwrap();
        store
            .submit_review(
                thread,
                reviewer,
                ReviewDecision::Approve,
                None,
                Some(&packet.evidence_root),
            )
            .await
            .unwrap();
        // Unlinked after the hand-off: the packet still pins it, the thread
        // no longer links it.
        store.unlink_thread_artifact(thread, DROPPED).await.unwrap();
        threads.push(thread);
        channels.push(channel.id);
    }
    let message = store
        .post_message(maidan_types::NewMessage {
            thread_id: threads[0],
            author_id: worker,
            body: "handed off".into(),
            metadata: json!({}),
            content: None,
        })
        .await
        .unwrap()
        .id;
    Tenant {
        workspace: ws,
        channel: channels[0],
        operator_id: operator,
        message,
        thread: threads[0],
        private_thread: threads[1],
        worker,
        reviewer,
        operator: signed_in(store, ws, operator).await,
        outsider: signed_in(store, ws, outsider).await,
    }
}

async fn get(client: &reqwest::Client, url: String, cookie: &str) -> (StatusCode, Value, String) {
    let resp = client
        .get(url)
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let text = resp.text().await.unwrap();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, body, content_type)
}

#[tokio::test]
async fn a_signed_in_member_reads_who_decided_and_who_linked_each_artifact() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let a = tenant(store.as_ref(), "a").await;
    let t = a.thread.0;

    let (s, reviews, _) = get(
        &client,
        format!("{base}/ui/api/threads/{t}/reviews"),
        &a.operator,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{reviews}");
    let reviews = reviews.as_array().unwrap();
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(reviews[0]["reviewer_id"], json!(a.reviewer.0));
    assert_eq!(reviews[0]["decision"], json!("approve"));
    assert!(reviews[0]["evidence_root"].is_string(), "{reviews:?}");

    let (s, links, _) = get(
        &client,
        format!("{base}/ui/api/threads/{t}/artifacts"),
        &a.operator,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{links}");
    let links = links.as_array().unwrap();
    assert_eq!(links.len(), 1, "only the artifact still linked: {links:?}");
    assert_eq!(links[0]["sha256"], json!(HELD));
    assert_eq!(links[0]["linked_by"], json!(a.worker.0));
    assert!(links[0]["linked_at"].is_string(), "{links:?}");
}

/// Who moved the task into review is a session read of the transition history.
/// The worker handed it off, so that member is the actor of the move into review.
#[tokio::test]
async fn a_signed_in_member_reads_who_requested_the_review() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let a = tenant(store.as_ref(), "a").await;

    let (s, rows, _) = get(
        &client,
        format!("{base}/ui/api/threads/{}/transitions", a.thread.0),
        &a.operator,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{rows}");
    let into_review: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["to_state"] == "in_review")
        .collect();
    assert_eq!(into_review.len(), 1, "{rows}");
    assert_eq!(into_review[0]["actor_id"], json!(a.worker.0));
    assert_eq!(into_review[0]["from_state"], "open");
}

/// The console links a file the workspace already holds onto the open task,
/// and unlinks it. Another workspace's session cannot.
#[tokio::test]
async fn a_signed_in_member_links_and_unlinks_a_file_on_the_open_task() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let a = tenant(store.as_ref(), "a").await;
    let b = tenant(store.as_ref(), "b").await;
    let t = a.thread.0;
    let link = format!("{base}/ui/api/threads/{t}/artifacts/{DROPPED}");

    let denied = client
        .put(&link)
        .header("Cookie", &b.operator)
        .header("Idempotency-Key", "other-workspace")
        .send()
        .await
        .unwrap();
    assert!(
        matches!(
            denied.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ),
        "another workspace linked A's file: {}",
        denied.status()
    );

    let linked = client
        .put(&link)
        .header("Cookie", &a.operator)
        .header("Idempotency-Key", "link-dropped")
        .send()
        .await
        .unwrap();
    assert_eq!(
        linked.status(),
        StatusCode::OK,
        "{}",
        linked.text().await.unwrap()
    );
    let (_s, rows, _) = get(
        &client,
        format!("{base}/ui/api/threads/{t}/artifacts"),
        &a.operator,
    )
    .await;
    let shas: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["sha256"].as_str().unwrap().to_string())
        .collect();
    assert!(shas.contains(&DROPPED.to_string()), "{shas:?}");

    let unlinked = client
        .delete(&link)
        .header("Cookie", &a.operator)
        .header("Idempotency-Key", "unlink-dropped")
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked.status(), StatusCode::NO_CONTENT);
    let (_s, rows, _) = get(
        &client,
        format!("{base}/ui/api/threads/{t}/artifacts"),
        &a.operator,
    )
    .await;
    let shas: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["sha256"].as_str())
        .collect();
    assert!(!shas.contains(&DROPPED), "{shas:?}");
}

/// Workspace B's signed-in session gets nothing of A's reviews or links: the
/// same refusal as the packet read beside them (403 or 404, a problem body
/// naming none of A's members or hashes), while B's own task reads fine. A
/// member outside A's private channel is refused the same way as the packet.
#[tokio::test]
async fn another_workspaces_session_gets_nothing_of_a_threads_reviews_or_links() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let a = tenant(store.as_ref(), "a").await;
    let b = tenant(store.as_ref(), "b").await;

    for tid in [a.thread.0, a.private_thread.0] {
        for read in ["reviews", "artifacts", "transitions", "review-packet"] {
            let (s, body, content_type) = get(
                &client,
                format!("{base}/ui/api/threads/{tid}/{read}"),
                &b.operator,
            )
            .await;
            assert!(
                matches!(s, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
                "B's session reads A's {read}: {s} {body}"
            );
            assert!(
                content_type.starts_with("application/problem+json"),
                "{read}: {content_type}"
            );
            let text = body.to_string();
            for secret in [
                a.reviewer.0.to_string(),
                a.worker.0.to_string(),
                HELD.to_string(),
            ] {
                assert!(!text.contains(&secret), "{read} leaked {secret}: {text}");
            }
        }
    }

    // The new reads answer as the packet read does, caller by caller.
    for (who, cookie, tid) in [
        ("A's operator, public", &a.operator, a.thread.0),
        ("A's operator, private", &a.operator, a.private_thread.0),
        ("A's outsider, public", &a.outsider, a.thread.0),
        ("A's outsider, private", &a.outsider, a.private_thread.0),
        ("B's operator, A's public", &b.operator, a.thread.0),
        ("B's operator, A's private", &b.operator, a.private_thread.0),
        ("B's operator, its own", &b.operator, b.thread.0),
    ] {
        let (packet, _, _) = get(
            &client,
            format!("{base}/ui/api/threads/{tid}/review-packet"),
            cookie,
        )
        .await;
        for read in ["reviews", "artifacts", "transitions"] {
            let (s, body, _) = get(
                &client,
                format!("{base}/ui/api/threads/{tid}/{read}"),
                cookie,
            )
            .await;
            assert_eq!(
                s, packet,
                "{who}: {read} answered {s} {body}, the packet {packet}"
            );
        }
    }
    let (s, _, _) = get(
        &client,
        format!("{base}/ui/api/threads/{}/reviews", a.private_thread.0),
        &a.outsider,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "outside the private channel, nothing"
    );

    // B's own reads succeed, so the refusals above are the tenant boundary.
    let (s, links, _) = get(
        &client,
        format!("{base}/ui/api/threads/{}/artifacts", b.thread.0),
        &b.operator,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(links[0]["linked_by"], json!(b.worker.0));
}

/// A token, and a session signed in with it: the session carries the token's
/// authority, so each `/ui/api` read must answer exactly as its bearer twin
/// (the same path without `/ui/api`) does for that token.
async fn token_and_session(
    store: &dyn Store,
    ws: WorkspaceId,
    member: MemberId,
) -> (String, String) {
    let secret = maidan_auth::TokenSecret::generate();
    let token = store
        .create_api_token(maidan_types::NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: maidan_auth::hash_secret(secret.as_str()),
            label: None,
            capabilities: [
                maidan_auth::capability::WORKSPACE_READ,
                maidan_auth::capability::SEARCH_QUERY,
                maidan_auth::capability::TOKEN_ADMIN,
                maidan_auth::capability::FEDERATION_ADMIN,
            ]
            .map(String::from)
            .to_vec(),
            expires_at: None,
        })
        .await
        .unwrap();
    let session = store
        .create_session(NewMaidanSession {
            workspace_id: ws,
            member_id: member,
            api_token_id: Some(token.id),
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
    let cookie = headers
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    (secret.as_str().to_string(), cookie)
}

/// Every `/ui/api` read route, filled in with one tenant's ids. A route added
/// to the session tree belongs here too.
fn ui_reads(t: &Tenant) -> Vec<String> {
    let (w, c, th, m, me) = (
        t.workspace.0,
        t.channel.0,
        t.thread.0,
        t.message.0,
        t.operator_id.0,
    );
    let unknown = uuid::Uuid::now_v7();
    [
        format!("/workspaces/{w}"),
        format!("/workspaces/{w}/events"),
        format!("/workspaces/{w}/channels"),
        format!("/workspaces/{w}/search?q=handed"),
        format!("/workspaces/{w}/audit"),
        format!("/workspaces/{w}/peers"),
        format!("/workspaces/{w}/group-dms?member_id={me}"),
        format!("/workspaces/{w}/dm?member_id={me}"),
        format!("/workspaces/{w}/slash-commands"),
        format!("/workspaces/{w}/deliveries"),
        format!("/workspaces/{w}/members"),
        format!("/workspaces/{w}/members/{me}/tokens"),
        format!("/workspaces/{w}/app-installations"),
        format!("/workspaces/{w}/approval-gates"),
        format!("/workspaces/{w}/task-schedules"),
        format!("/channels/{c}/threads"),
        format!("/channels/{c}/queue-depth"),
        format!("/channels/{c}/occupancy"),
        format!("/threads/{th}"),
        format!("/threads/{th}/messages"),
        format!("/threads/{th}/review-status"),
        format!("/threads/{th}/review-packet"),
        format!("/threads/{th}/reviews"),
        format!("/threads/{th}/artifacts"),
        format!("/threads/{th}/pins"),
        format!("/threads/{th}/result"),
        format!("/threads/{th}/dependencies"),
        format!("/messages/{m}/edits"),
        format!("/messages/{m}/reactions"),
        format!("/group-dms/{unknown}"),
        format!("/members/{me}/notifications"),
        format!("/members/{me}/notifications/unread-count"),
        format!("/members/{me}/notification-prefs"),
        format!("/members/{me}/delivery-mode"),
        format!("/members/{me}/email"),
        format!("/members/{me}/channel-follows"),
        format!("/members/{me}/thread-follows"),
        format!("/members/{me}/waiting"),
        format!("/artifacts/{HELD}/meta"),
        format!("/artifacts/{HELD}"),
        "/me".to_string(),
        "/web-push/vapid-public-key".to_string(),
    ]
    .into()
}

async fn get_bearer(client: &reqwest::Client, url: String, token: &str) -> (StatusCode, String) {
    let resp = client.get(url).bearer_auth(token).send().await.unwrap();
    (resp.status(), resp.text().await.unwrap())
}

async fn get_cookie(client: &reqwest::Client, url: String, cookie: &str) -> (StatusCode, String) {
    let resp = client
        .get(url)
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap();
    (resp.status(), resp.text().await.unwrap())
}

/// The session tree serves the bearer tree's handlers: for each `/ui/api`
/// read, a session holding a token's authority gets the status and body the
/// token gets from the twin route, for its own workspace's ids and for
/// another workspace's, which both refuse with nothing of B's in the body.
#[tokio::test]
async fn every_ui_api_read_answers_as_its_bearer_twin_and_refuses_another_workspace() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let a = tenant(store.as_ref(), "a").await;
    let b = tenant(store.as_ref(), "b").await;
    let (token, cookie) = token_and_session(store.as_ref(), a.workspace, a.operator_id).await;

    let own = ui_reads(&a);
    let mut not_ok = Vec::new();
    for path in &own {
        let (session_status, session_body) =
            get_cookie(&client, format!("{base}/ui/api{path}"), &cookie).await;
        let (bearer_status, bearer_body) =
            get_bearer(&client, format!("{base}{path}"), &token).await;
        assert_eq!(
            session_status, bearer_status,
            "{path}: session {session_status} {session_body}, bearer {bearer_status} {bearer_body}"
        );
        assert_eq!(session_body, bearer_body, "{path}");
        if session_status != StatusCode::OK {
            not_ok.push(path.clone());
        }
    }
    // The rest answer 200, so the comparison above covers real bodies. These
    // four name nothing that exists: no email is set, the artifact is a bare
    // reference with no blob, and the group DM id is made up.
    let expected_missing = [
        format!("/members/{}/email", a.operator_id.0),
        format!("/artifacts/{HELD}/meta"),
        format!("/artifacts/{HELD}"),
    ];
    not_ok.retain(|p| !expected_missing.contains(p) && !p.starts_with("/group-dms/"));
    assert!(not_ok.is_empty(), "reads that did not answer: {not_ok:#?}");

    let b_ids = [
        b.workspace.0.to_string(),
        b.channel.0.to_string(),
        b.thread.0.to_string(),
        b.message.0.to_string(),
        b.operator_id.0.to_string(),
    ];
    for path in ui_reads(&b) {
        // Paths that name no workspace's ids answer the caller's own data.
        if !b_ids.iter().any(|id| path.contains(id.as_str())) {
            continue;
        }
        let (session_status, session_body) =
            get_cookie(&client, format!("{base}/ui/api{path}"), &cookie).await;
        let (bearer_status, _) = get_bearer(&client, format!("{base}{path}"), &token).await;
        assert!(
            matches!(
                session_status,
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ),
            "A's session reads B's {path}: {session_status} {session_body}"
        );
        assert_eq!(session_status, bearer_status, "{path}");
        for id in &b_ids {
            if !path.contains(id.as_str()) {
                assert!(
                    !session_body.contains(id.as_str()),
                    "{path} leaked {id}: {session_body}"
                );
            }
        }
    }
}

/// A signed-in member sets and clears their own display name. Another member,
/// including one in another workspace, cannot.
#[tokio::test]
async fn a_signed_in_member_sets_and_clears_their_own_display_name() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let a = tenant(store.as_ref(), "a").await;
    let b = tenant(store.as_ref(), "b").await;
    let mine = format!("{base}/ui/api/members/{}", a.operator_id.0);

    let set = client
        .patch(&mine)
        .header("Cookie", &a.operator)
        .header("Idempotency-Key", "set-name")
        .header("Content-Type", "application/json")
        .body(r#"{"display_name":"Ada Operator"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(
        set.status(),
        StatusCode::OK,
        "{}",
        set.text().await.unwrap()
    );

    let (s, member, _) = get(&client, mine.clone(), &a.operator).await;
    assert_eq!(s, StatusCode::OK, "{member}");
    assert_eq!(member["display_name"], json!("Ada Operator"));

    let foreign = client
        .patch(&mine)
        .header("Cookie", &b.operator)
        .header("Idempotency-Key", "other-workspace-name")
        .header("Content-Type", "application/json")
        .body(r#"{"display_name":"Stolen"}"#)
        .send()
        .await
        .unwrap();
    assert!(
        matches!(
            foreign.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ),
        "another workspace renamed A's member: {}",
        foreign.status()
    );

    let someone_else = format!("{base}/ui/api/members/{}", a.worker.0);
    let not_yours = client
        .patch(&someone_else)
        .header("Cookie", &a.operator)
        .header("Idempotency-Key", "not-yours")
        .header("Content-Type", "application/json")
        .body(r#"{"display_name":"Not yours"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(not_yours.status(), StatusCode::FORBIDDEN);

    let cleared = client
        .patch(&mine)
        .header("Cookie", &a.operator)
        .header("Idempotency-Key", "clear-name")
        .header("Content-Type", "application/json")
        .body(r#"{"display_name":"  "}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(
        cleared.status(),
        StatusCode::OK,
        "{}",
        cleared.text().await.unwrap()
    );
    let (s, member, _) = get(&client, mine, &a.operator).await;
    assert_eq!(s, StatusCode::OK, "{member}");
    assert!(member["display_name"].is_null(), "{member}");
}

/// `base` with a unique suffix, so fixture tasks created alike in one
/// workspace never share a fingerprint: a live duplicate is not claimable
/// (Context Economics C8b), and these tests are not about duplicates.
fn distinct_brief(base: &str) -> String {
    format!("{base} {}", uuid::Uuid::now_v7())
}
