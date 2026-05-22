//! `/health` endpoint. Reports the status of every external dependency
//! the server needs to be useful — currently the DB and the artifact
//! store. Returns 200 only when both subsystems respond.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use maidan_artifacts::Sha256;
use serde::Serialize;

use crate::state::AppState;
use crate::version;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub db: SubsystemStatus,
    pub storage: SubsystemStatus,
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

pub async fn handler(State(state): State<AppState>) -> impl IntoResponse {
    let db = match state.store.health_check().await {
        Ok(()) => SubsystemStatus::Ok,
        Err(e) => SubsystemStatus::Error(e.to_string()),
    };

    let storage = check_artifact_store(&state).await;

    let healthy = db.is_ok() && storage.is_ok();
    let body = HealthResponse {
        status: if healthy { "ok" } else { "degraded" },
        db,
        storage,
        version: version(),
    };
    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(body))
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
