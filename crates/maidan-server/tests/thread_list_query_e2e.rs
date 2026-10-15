//! The thread list's order and filters over REST: `order=newest` reverses
//! the keyset walk, `since` bounds by creation time, and `result_kind` /
//! `status` filter before the limit — the reads an agent dashboard makes
//! instead of fetching five hundred threads to find today's.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    DeclaredStatus, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    base: String,
    _server: tokio::task::JoinHandle<()>,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

async fn spawn() -> Ctx {
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
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Ctx {
        base: format!("http://{addr}"),
        _server: server,
        store,
        _dir: dir,
    }
}

async fn mint(
    store: &dyn Store,
    ws: maidan_types::WorkspaceId,
    member: maidan_types::MemberId,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn list(
    ctx: &Ctx,
    token: &str,
    channel: maidan_types::ChannelId,
    query: &str,
) -> (StatusCode, Value) {
    let resp = reqwest::Client::new()
        .get(format!(
            "{}/channels/{}/threads?{}",
            ctx.base, channel.0, query
        ))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

fn ids_of(body: &Value) -> Vec<String> {
    body.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn the_thread_list_orders_and_filters_over_rest() {
    let ctx = spawn().await;
    let store = ctx.store.as_ref();
    let ws = store
        .create_workspace(NewWorkspace {
            name: "listq".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "reader".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let token = mint(store, ws.id, member.id).await;
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let mut ids = Vec::new();
    for title in ["one", "two", "three", "four"] {
        let t = store
            .create_thread(NewThread {
                channel_id: ch.id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
            })
            .await
            .unwrap();
        ids.push(t.id);
    }
    let review = json!({ "result_kind": "example.review.result/1", "verdict": "ok" });
    store
        .set_thread_result(ids[1], member.id, &review)
        .await
        .unwrap();
    store
        .set_thread_result(ids[3], member.id, &review)
        .await
        .unwrap();
    for (id, status) in [
        (ids[1], DeclaredStatus::Working),
        (ids[3], DeclaredStatus::Done),
    ] {
        store.claim_thread(id, member.id).await.unwrap();
        store
            .declare_thread_status(id, status, "note".into(), member.id)
            .await
            .unwrap();
    }
    let ids: Vec<String> = ids.iter().map(|i| i.0.to_string()).collect();

    // Default: creation order.
    let (s, body) = list(&ctx, &token, ch.id, "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), ids);

    // order=newest: the exact reverse, and its cursor continues the
    // reversed walk.
    let (s, body) = list(&ctx, &token, ch.id, "order=newest").await;
    assert_eq!(s, StatusCode::OK);
    let mut reversed = ids.clone();
    reversed.reverse();
    assert_eq!(ids_of(&body), reversed);
    let (s, body) = list(&ctx, &token, ch.id, "order=newest&limit=3").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), reversed[..3].to_vec());
    let cursor = reversed[2].clone();
    let (s, body) = list(
        &ctx,
        &token,
        ch.id,
        &format!("order=newest&cursor={cursor}"),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), reversed[3..].to_vec());

    // since: the third thread's creation instant keeps it and the fourth.
    let third = store
        .get_thread(maidan_types::ThreadId(
            uuid::Uuid::parse_str(&ids[2]).unwrap(),
        ))
        .await
        .unwrap();
    let since = urlencoding::encode(&third.created_at.to_rfc3339()).into_owned();
    let (s, body) = list(&ctx, &token, ch.id, &format!("since={since}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), ids[2..].to_vec());

    // result_kind and status, alone and composed, in both orders.
    let (s, body) = list(&ctx, &token, ch.id, "result_kind=example.review.result/1").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), vec![ids[1].clone(), ids[3].clone()]);
    let (s, body) = list(
        &ctx,
        &token,
        ch.id,
        "result_kind=example.review.result/1&order=newest",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), vec![ids[3].clone(), ids[1].clone()]);
    let (s, body) = list(&ctx, &token, ch.id, "status=done").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), vec![ids[3].clone()]);
    let (s, body) = list(
        &ctx,
        &token,
        ch.id,
        "status=working&result_kind=example.review.result/1",
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(ids_of(&body), vec![ids[1].clone()]);

    // Garbage values are a 400, not a silent fall back to the default view.
    for query in ["order=sideways", "status=bogus", "since=not-a-date"] {
        let (s, _) = list(&ctx, &token, ch.id, query).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "query {query}");
    }
}
