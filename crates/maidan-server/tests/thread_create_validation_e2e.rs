//! `POST /channels/{id}/threads`: a blank title is a 422, unknown fields are
//! rejected, and `description` is accepted and persisted.

use std::sync::Arc;
use std::time::Duration;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    base: String,
    client: reqwest::Client,
    member_id: String,
    channel_id: String,
}

async fn setup() -> Ctx {
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
    let artifacts = Arc::new(LocalFsStore::new(dir.keep()));
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ws = store
        .create_workspace(NewWorkspace {
            name: "desc-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "op".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "ch".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Ctx {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        member_id: member.id.0.to_string(),
        channel_id: ch.id.0.to_string(),
    }
}

fn post(ctx: &Ctx, body: Value) -> reqwest::RequestBuilder {
    ctx.client
        .post(format!("{}/channels/{}/threads", ctx.base, ctx.channel_id))
        .header("maidan-test-member-id", &ctx.member_id)
        .json(&body)
}

#[tokio::test]
async fn blank_title_is_422() {
    let ctx = setup().await;
    for blank in [json!({"title": ""}), json!({"title": "   "})] {
        let res = post(&ctx, blank).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let problem: Value = res.json().await.unwrap();
        assert!(
            problem["detail"].as_str().unwrap().contains("blank"),
            "{problem}"
        );
    }
}

#[tokio::test]
async fn null_title_is_an_untitled_thread() {
    let ctx = setup().await;
    for body in [json!({}), json!({"title": null})] {
        let res = post(&ctx, body).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED, "{res:?}");
        let thread: Value = res.json().await.unwrap();
        assert!(thread["title"].is_null());
    }
}

#[tokio::test]
async fn unknown_fields_are_rejected() {
    let ctx = setup().await;
    let res = post(&ctx, json!({"title": "t", "bogus": 1}))
        .send()
        .await
        .unwrap();
    // The extractor normalizes axum's 422 to 400: one status for "cannot read".
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn description_is_accepted_and_persisted() {
    let ctx = setup().await;
    let res = post(&ctx, json!({"title": "t", "description": "do the thing"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let thread: Value = res.json().await.unwrap();
    assert_eq!(thread["description"], "do the thing");
    let id = thread["id"].as_str().unwrap();

    // A fresh read shows the description too.
    let got: Value = ctx
        .client
        .get(format!("{}/threads/{id}", ctx.base))
        .header("maidan-test-member-id", &ctx.member_id)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["description"], "do the thing");
}

#[tokio::test]
async fn threads_without_description_omit_it() {
    let ctx = setup().await;
    let res = post(&ctx, json!({"title": "t"})).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let thread: Value = res.json().await.unwrap();
    assert!(thread.get("description").is_none(), "{thread}");
}
