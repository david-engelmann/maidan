//! The approval card's two session-proxied thread reads, with auth on and two
//! workspaces: `GET /ui/api/threads/{tid}/reviews` (who decided, after a
//! reload) and `GET /ui/api/threads/{tid}/artifacts` (who linked each artifact
//! and when). They serve the bearer tree's handlers, so they answer exactly
//! when the packet read does: a signed-in member of the thread's workspace
//! reads them, another workspace's session gets nothing of them, and neither
//! does a member outside a private channel.

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
                title: Some("Ship it".into()),
                description: None,
            })
            .await
            .unwrap()
            .id;
        store.claim_thread(thread, worker).await.unwrap();
        store
            .set_thread_result(thread, worker, &json!({"status": "done"}))
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
    }
    Tenant {
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
        for read in ["reviews", "artifacts", "review-packet"] {
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
        for read in ["reviews", "artifacts"] {
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
