//! `/health` endpoint. Reports the status of every external dependency
//! the server needs to be useful — currently the DB and the artifact
//! store. Returns 200 only when both subsystems respond.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use maidan_artifacts::Sha256;
use serde::Serialize;

use crate::state::AppState;
use crate::version;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub db: SubsystemStatus,
    pub storage: SubsystemStatus,
    pub indexer: SubsystemStatus,
    pub indexer_last_event_at: Option<DateTime<Utc>>,
    pub bus: SubsystemStatus,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemStatus {
    Ok,
    Error(String),
}

impl SubsystemStatus {
    fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

pub async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

pub async fn ready(state: State<AppState>) -> impl IntoResponse {
    readiness(state).await
}

pub async fn handler(state: State<AppState>) -> impl IntoResponse {
    readiness(state).await
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let db = match state.store.health_check().await {
        Ok(()) => SubsystemStatus::Ok,
        Err(e) => SubsystemStatus::Error(e.to_string()),
    };

    let storage = check_artifact_store(&state).await;
    let (indexer, indexer_last_event_at) = check_indexer(&state);
    let bus = check_bus(&state);

    let healthy = db.is_ok() && storage.is_ok() && indexer.is_ok() && bus.is_ok();
    let body = HealthResponse {
        status: if healthy { "ok" } else { "degraded" },
        db,
        storage,
        indexer,
        indexer_last_event_at,
        bus,
        version: version(),
    };
    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(body))
}

fn check_bus(state: &AppState) -> SubsystemStatus {
    match &state.bus_listener_health {
        None => SubsystemStatus::Ok,
        Some(health) => match health.check() {
            Ok(()) => SubsystemStatus::Ok,
            Err(msg) => SubsystemStatus::Error(msg),
        },
    }
}

fn check_indexer(state: &AppState) -> (SubsystemStatus, Option<DateTime<Utc>>) {
    let ms = state.indexer_last_event_unix_ms.load(Ordering::Relaxed);
    if ms == 0 {
        return (SubsystemStatus::Ok, None);
    }
    let at = DateTime::from_timestamp_millis(ms);
    let stale_secs = std::env::var("INDEXER_STALE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if stale_secs > 0 {
        let age_secs = (Utc::now().timestamp_millis() - ms) / 1000;
        if age_secs > stale_secs {
            return (
                SubsystemStatus::Error(format!(
                    "no indexer activity for {age_secs}s (threshold {stale_secs}s)"
                )),
                at,
            );
        }
    }
    (SubsystemStatus::Ok, at)
}

async fn check_artifact_store(state: &AppState) -> SubsystemStatus {
    // Use a deterministic probe sha that we never write; existence check
    // is sufficient to verify the backend is reachable and not panicking.
    let probe = Sha256::compute(b"maidan-health-probe");
    match state.artifacts.exists(&probe).await {
        Ok(_) => SubsystemStatus::Ok,
        Err(e) => SubsystemStatus::Error(e.to_string()),
    }
}
