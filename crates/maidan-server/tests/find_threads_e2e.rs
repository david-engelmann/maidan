//! Find threads by attribute over the wire: `GET /workspaces/{id}/threads`
//! matches on the threads' task attributes, is scoped to the caller's own
//! workspace, refuses a foreign workspace and a garbage filter, and pages.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (String, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), store)
}

const CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::THREAD_TRANSITION,
];

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    token: String,
}

async fn room(base_store: &dyn Store, name: &str) -> Room {
    let ws = base_store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = base_store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    base_store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: CAPS.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    let channel = base_store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        token: secret.as_str().to_string(),
    }
}

async fn create_thread(base: &str, room: &Room, body: Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn find(base: &str, token: &str, ws: WorkspaceId, query: &str) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}/workspaces/{}/threads?{query}", ws.0))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

fn ids(body: &Value) -> Vec<String> {
    body.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn find_threads_by_attribute_over_the_wire() {
    let (base, store) = spawn().await;
    let a = room(store.as_ref(), "find-a").await;
    let b = room(store.as_ref(), "find-b").await;

    let pi_attrs = json!({
        "kind": "change", "engine": "pi",
        "linear_issue": "ENG-100", "tier": "A", "lane": "batch"
    });
    let (status, pi) = create_thread(
        &base,
        &a,
        json!({ "title": "pi issue", "attributes": pi_attrs }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pi}");
    let (status, session) = create_thread(
        &base,
        &a,
        json!({
            "title": "session issue",
            "attributes": { "kind": "change", "engine": "session", "linear_issue": "ENG-200", "tier": "B" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let (status, plain) = create_thread(&base, &a, json!({ "title": "plain" })).await;
    assert_eq!(status, StatusCode::CREATED, "{plain}");
    // The same Linear issue in the other workspace never leaks into A's
    // results, and A's token cannot search B at all.
    let (status, other) = create_thread(
        &base,
        &b,
        json!({ "title": "other pi issue", "attributes": pi_attrs }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");

    let (status, found) = find(&base, &a.token, a.ws, "attr.linear_issue=ENG-100").await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(ids(&found), vec![pi["id"].as_str().unwrap()]);
    assert_eq!(found[0]["attributes"]["engine"], "pi");
    assert_eq!(found[0]["attributes"]["tier"], "A");

    // The bare shorthand agrees with the attr. spelling.
    let (status, found) = find(&base, &a.token, a.ws, "engine=pi").await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(ids(&found), vec![pi["id"].as_str().unwrap()]);

    // An unset field matches nothing: the plain thread has no tier.
    let (status, found) = find(&base, &a.token, a.ws, "attr.tier=B").await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(ids(&found), vec![session["id"].as_str().unwrap()]);

    // The default a thread reads with does match: kind=change takes the
    // described change threads and the undescribed one.
    let (status, found) = find(&base, &a.token, a.ws, "attr.kind=change").await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found.as_array().unwrap().len(), 3);

    // B finds its own thread with the same filter.
    let (status, found) = find(&base, &b.token, b.ws, "attr.linear_issue=ENG-100").await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(ids(&found), vec![other["id"].as_str().unwrap()]);

    // ref.<relation> keeps the threads holding that reference: session is
    // the twin of pi. B's search for twins of A's thread finds nothing, so
    // the filter cannot probe another workspace's threads.
    let id = |t: &Value| uuid::Uuid::parse_str(t["id"].as_str().unwrap()).unwrap();
    store
        .add_reference(maidan_types::NewReference {
            src_kind: maidan_types::RefSide::Thread,
            src_id: id(&session),
            dst_kind: maidan_types::RefSide::Thread,
            dst_id: id(&pi),
            relation: maidan_types::RelationKind::from_wire("twin_of"),
        })
        .await
        .unwrap();
    let twins = format!("ref.twin_of={}", id(&pi));
    let (status, found) = find(&base, &a.token, a.ws, &twins).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(ids(&found), vec![session["id"].as_str().unwrap()]);
    let (status, found) = find(
        &base,
        &a.token,
        a.ws,
        &format!("ref.duplicates={}", id(&pi)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert!(ids(&found).is_empty(), "{found}");
    let (status, found) = find(&base, &b.token, b.ws, &twins).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert!(ids(&found).is_empty(), "{found}");

    // A's token searching B's workspace is refused, and no filter value
    // changes that.
    let (status, _) = find(&base, &a.token, b.ws, "attr.linear_issue=ENG-100").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Garbage refuses rather than silently matching everything.
    for query in [
        "attr.engine=robot",
        "attr.engnie=pi",
        "attr.linear_issue=not-a-key",
        "ref.twin_of=not-a-thread",
        "ref.=00000000-0000-0000-0000-000000000000",
        "attr.twin_of=00000000-0000-0000-0000-000000000000",
        "bogus=1",
        "order=sideways",
    ] {
        let (status, body) = find(&base, &a.token, a.ws, query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
    }

    // Keyset pagination: one at a time through the kind=change matches.
    let (status, page1) = find(&base, &a.token, a.ws, "attr.kind=change&limit=2").await;
    assert_eq!(status, StatusCode::OK, "{page1}");
    assert_eq!(page1.as_array().unwrap().len(), 2);
    let cursor = page1[1]["id"].as_str().unwrap();
    let (status, page2) = find(
        &base,
        &a.token,
        a.ws,
        &format!("attr.kind=change&limit=2&cursor={cursor}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page2}");
    assert_eq!(page2.as_array().unwrap().len(), 1);
    assert_eq!(page2[0]["id"], plain["id"]);
}
