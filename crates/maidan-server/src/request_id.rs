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

/// Accept only a trimmed inbound id of 1..=128 safe ASCII characters. Anything
/// else (absent, blank, overlong, control/whitespace, log-injection) is replaced
/// with a fresh UUIDv7 so response headers and traces stay printable.
pub(crate) fn sanitize_request_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub async fn middleware(mut req: Request, next: Next) -> Response {
    let header = request_id_header();
    let id = req
        .headers()
        .get(&header)
        .and_then(|v| v.to_str().ok())
        .and_then(sanitize_request_id)
        .unwrap_or_else(|| Uuid::now_v7().to_string());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_trimmed_safe_ids() {
        assert_eq!(
            sanitize_request_id("  abc-123_./:x  ").as_deref(),
            Some("abc-123_./:x")
        );
    }

    #[test]
    fn rejects_blank_overlong_and_unsafe() {
        assert!(sanitize_request_id("").is_none());
        assert!(sanitize_request_id("   ").is_none());
        assert!(sanitize_request_id(&"a".repeat(129)).is_none());
        assert!(sanitize_request_id("has space").is_none());
        assert!(sanitize_request_id("bad\ninject").is_none());
        assert!(sanitize_request_id("emoji-😀").is_none());
        assert!(sanitize_request_id("tab\there").is_none());
    }
}
