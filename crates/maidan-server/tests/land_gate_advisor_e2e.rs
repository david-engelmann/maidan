//! The experimental advisor is a separate, default-off read path: it returns
//! model advice but cannot create or change the authoritative land-gate row.

use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{
    land_gate_advisor::{
        LandGateAdvice, LandGateAdviceRequest, LandGateAdviceThresholds, LandGateAdviceUsage,
        LandGateAdvisor, LandGateAdvisorError,
    },
    router, AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{LandColor, NewChannel, NewThread, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

struct MockAdvisor;
struct FailingAdvisor;

#[async_trait::async_trait]
impl LandGateAdvisor for MockAdvisor {
    async fn advise(
        &self,
        request: LandGateAdviceRequest,
    ) -> Result<LandGateAdvice, LandGateAdvisorError> {
        assert_eq!(request.state["change"], "bounded fixture");
        Ok(LandGateAdvice {
            provider: "fixture".into(),
            model: "fixture-choice".into(),
            raw_land: LandColor::Green,
            recommended_land: LandColor::Amber,
            confidence: 0.81,
            probabilities: BTreeMap::from([
                ("green".into(), 0.81),
                ("amber".into(), 0.14),
                ("red".into(), 0.05),
            ]),
            thresholds: LandGateAdviceThresholds {
                green_min_confidence: 0.9,
                red_min_confidence: 0.9,
            },
            usage: LandGateAdviceUsage {
                input_tokens: 100,
                output_tokens: 4,
            },
            latency_ms: 17,
        })
    }
}

#[async_trait::async_trait]
impl LandGateAdvisor for FailingAdvisor {
    async fn advise(
        &self,
        _request: LandGateAdviceRequest,
    ) -> Result<LandGateAdvice, LandGateAdvisorError> {
        Err(LandGateAdvisorError::Unavailable("fixture outage".into()))
    }
}

async fn spawn(advisor: Option<Arc<dyn LandGateAdvisor>>) -> (SocketAddr, Arc<dyn Store>) {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    if let Some(advisor) = advisor {
        state.attach_land_gate_advisor(advisor);
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, store)
}

async fn thread(store: &dyn Store) -> maidan_types::Thread {
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "advisor".into(),
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: workspace.id,
            name: "review".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("evaluate me".into()),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn advisor_is_off_by_default() {
    let (addr, store) = spawn(None).await;
    let thread = thread(store.as_ref()).await;
    let response = reqwest::Client::new()
        .post(format!(
            "http://{addr}/threads/{}/land-gate/advice",
            thread.id.0
        ))
        .json(&json!({"state": {"change": "bounded fixture"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn advice_reports_measurement_fields_without_writing_the_gate() {
    let (addr, store) = spawn(Some(Arc::new(MockAdvisor))).await;
    let thread = thread(store.as_ref()).await;
    let client = reqwest::Client::new();
    let advice = client
        .post(format!(
            "http://{addr}/threads/{}/land-gate/advice",
            thread.id.0
        ))
        .json(&json!({"state": {"change": "bounded fixture"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(advice.status(), StatusCode::OK);
    let body: Value = advice.json().await.unwrap();
    assert_eq!(body["raw_land"], "green");
    assert_eq!(body["recommended_land"], "amber");
    assert_eq!(body["confidence"], 0.81);
    assert_eq!(body["latency_ms"], 17);
    assert_eq!(body["usage"]["input_tokens"], 100);

    let standing = store.get_land_gate_standing(thread.id).await.unwrap();
    assert!(!standing.required);
    assert!(standing.pointer.is_none());
    assert!(standing.landable);
}

#[tokio::test]
async fn provider_failure_is_scoped_to_advice_and_does_not_arm_the_gate() {
    let (addr, store) = spawn(Some(Arc::new(FailingAdvisor))).await;
    let thread = thread(store.as_ref()).await;
    let response = reqwest::Client::new()
        .post(format!(
            "http://{addr}/threads/{}/land-gate/advice",
            thread.id.0
        ))
        .json(&json!({"state": {"change": "bounded fixture"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert!(body["detail"]
        .as_str()
        .unwrap()
        .contains("authoritative gate is unchanged"));

    let standing = store.get_land_gate_standing(thread.id).await.unwrap();
    assert!(!standing.required);
    assert!(standing.pointer.is_none());
}
