//! The environment a process was started with, said on every response.
//!
//! The hosted Worker starts the container with the Worker's settings and a
//! hash of them in `MAIDAN_ENV_FINGERPRINT`. An image rollout can restart the
//! container on the previous start's settings, so a setting that arrived in
//! the same deploy never reaches the process (it happened with
//! `MAIDAN_SIGNUP_OPEN` in v445). The Worker compares this header with the
//! hash it expects and recycles a container that answers with another one.
//! Without the variable no header is sent, so a self-hosted server is
//! unchanged.

use std::sync::OnceLock;

use axum::{http::HeaderValue, response::Response};

/// The response header that carries the fingerprint.
pub const HEADER: &str = "x-maidan-env-fingerprint";

/// A SHA-256 hex digest, or nothing: anything else is not the Worker's and is
/// not echoed.
fn parse(raw: Option<String>) -> Option<HeaderValue> {
    let raw = raw?;
    let raw = raw.trim();
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    HeaderValue::from_str(raw).ok()
}

fn value() -> Option<&'static HeaderValue> {
    static VALUE: OnceLock<Option<HeaderValue>> = OnceLock::new();
    VALUE
        .get_or_init(|| parse(std::env::var("MAIDAN_ENV_FINGERPRINT").ok()))
        .as_ref()
}

/// Stamp the fingerprint on a response, when the process has one.
pub async fn stamp(mut response: Response) -> Response {
    if let Some(value) = value() {
        response.headers_mut().insert(HEADER, value.clone());
    }
    response
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn only_a_sha256_hex_digest_is_echoed() {
        let digest = "a".repeat(64);
        let echoed = parse(Some(digest.clone())).map(|v| v.as_bytes().to_vec());
        assert_eq!(echoed, Some(digest.into_bytes()));
        assert!(parse(None).is_none());
        assert!(parse(Some(String::new())).is_none());
        assert!(parse(Some("a".repeat(63))).is_none());
        assert!(parse(Some(format!("{}g", "a".repeat(63)))).is_none());
        assert!(parse(Some("x\r\nset-cookie: a".into())).is_none());
    }
}
