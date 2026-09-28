//! Health probe path docs.

use crate::error::ProblemDetails;
use crate::health::HealthResponse;
use crate::openapi::responses::*;
use crate::openapi::schemas::LivenessOk;

/// Liveness probe (always OK when process is up).
#[utoipa::path(
    get,
    path = "/health/live",
    tag = "health",
    security(()),
    responses(
        (status = 200, description = "Alive", body = LivenessOk),
    )
)]
pub fn health_live() {}

/// Readiness probe (DB, storage, indexer, bus).
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    security(()),
    responses(
        (status = 200, description = "Ready", body = HealthResponse),
        (status = 503, description = "Degraded", body = HealthResponse),
    )
)]
pub fn health_ready() {}

/// Alias of readiness (`/health`).
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    security(()),
    responses(
        (status = 200, description = "Ready", body = HealthResponse),
        (status = 503, description = "Degraded", body = HealthResponse),
    )
)]
pub fn health() {}
