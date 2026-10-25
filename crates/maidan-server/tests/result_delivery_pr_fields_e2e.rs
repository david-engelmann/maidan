//! The delivery listing names the pull request a change landed as.
//!
//! A `github_branch` row's stored handle is `<commit_sha>#<pull_number>`;
//! the read view decodes it into `pr_number`, `head_sha` and `pr_url`
//! alongside the raw `external_ref`. Every other surface reads the three
//! fields as null. Rows are seeded straight through the store — what is
//! under test is the serialized listing, not the send path.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EgressTarget, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace,
    ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

struct Harness {
    store: Arc<dyn Store>,
    base: String,
    bearer: String,
    thread_id: ThreadId,
}

async fn mint(store: &dyn Store, ws: WorkspaceId, member: maidan_types::MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn harness() -> Harness {
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let ws = store
        .create_workspace(NewWorkspace {
            name: "pr-fields".into(),
        })
        .await
        .unwrap();
    let member = store
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
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("change".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
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
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        store,
        base: format!("http://{addr}"),
        bearer: format!("Bearer {tok}"),
        thread_id: thread.id,
    }
}

/// Arm a delivery for `target` and record it as delivered with `handle`.
async fn delivered(h: &Harness, target: &EgressTarget, handle: &str) {
    let rev = Utc::now();
    let row = h
        .store
        .arm_result_delivery(h.thread_id, target, rev)
        .await
        .unwrap()
        .unwrap();
    h.store
        .mark_result_delivered(row.id, Some(handle), rev)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_listing_decodes_a_branch_handle_and_nulls_the_rest() {
    let h = harness().await;
    delivered(
        &h,
        &EgressTarget::GithubBranch {
            repo: "acme/widgets".into(),
            branch: "feature".into(),
        },
        &format!("{SHA}#42"),
    )
    .await;
    delivered(
        &h,
        &EgressTarget::Github {
            repo: "acme/widgets".into(),
            issue_number: 7,
        },
        "998877",
    )
    .await;
    delivered(
        &h,
        &EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into(),
            thread_ts: None,
        },
        "1699999999.001200",
    )
    .await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/threads/{}/deliveries", h.base, h.thread_id.0))
        .header("Authorization", &h.bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let by_surface = |surface: &str| {
        rows.iter()
            .find(|r| r["surface"] == json!(surface))
            .unwrap_or_else(|| panic!("no {surface} row in {body}"))
    };

    let branch = by_surface("github_branch");
    assert_eq!(branch["external_ref"], json!(format!("{SHA}#42")));
    assert_eq!(branch["pr_number"], json!(42));
    assert_eq!(branch["head_sha"], json!(SHA));
    assert_eq!(
        branch["pr_url"],
        json!("https://github.com/acme/widgets/pull/42")
    );

    for surface in ["github", "slack"] {
        let row = by_surface(surface);
        assert_eq!(row["pr_number"], Value::Null, "{surface} pr_number");
        assert_eq!(row["head_sha"], Value::Null, "{surface} head_sha");
        assert_eq!(row["pr_url"], Value::Null, "{surface} pr_url");
    }
}
