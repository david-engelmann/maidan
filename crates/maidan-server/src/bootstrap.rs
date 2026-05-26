//! Bootstrap route gate: unauthenticated seed paths when auth is enabled.

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::error::ApiError;
use crate::state::AppState;

pub fn bootstrap_enabled_from_env() -> bool {
    matches!(
        std::env::var("MAIDAN_BOOTSTRAP").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

pub async fn middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.auth_disabled || state.bootstrap_enabled {
        return next.run(req).await;
    }
    ApiError::Forbidden("bootstrap routes require MAIDAN_BOOTSTRAP=1".into()).into_response()
}
