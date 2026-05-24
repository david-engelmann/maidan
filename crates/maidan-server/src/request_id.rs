//! Propagate or assign `X-Request-Id` on every HTTP response.

use axum::{
    extract::Request,
    http::{header::HeaderValue, HeaderName},
    middleware::Next,
    response::Response,
};
use tracing::Instrument;
use uuid::Uuid;

fn request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

pub async fn middleware(mut req: Request, next: Next) -> Response {
    let header = request_id_header();
    let id = req
        .headers()
        .get(&header)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    req.extensions_mut().insert(RequestId(id.clone()));

    let span = tracing::info_span!("http_request", request_id = %id);
    let mut response = async move { next.run(req).await }.instrument(span).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(header, value);
    }
    response
}

#[derive(Debug, Clone)]
pub struct RequestId(pub String);
