//! The OAuth error envelope of the token and revocation endpoints.
//!
//! Every other route answers a client error with an RFC 9457 problem. These
//! two are called by OAuth clients that parse RFC 6749 section 5.2 errors,
//! `{"error": "...", "error_description": "..."}`, and the RFC 8414 document
//! advertises them to such clients, so they answer in that envelope instead,
//! as SCIM and A2A answer in theirs.

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiError;

/// An RFC 6749 section 5.2 error code, plus RFC 8707's `invalid_target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthErrorCode {
    InvalidRequest,
    InvalidClient,
    InvalidGrant,
    UnauthorizedClient,
    UnsupportedGrantType,
    InvalidScope,
    InvalidTarget,
    /// RFC 6749 section 4.1.2.1's code for an unexpected failure, used here
    /// so a 500 from these endpoints is still in the envelope clients parse.
    ServerError,
}

impl OAuthErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidClient => "invalid_client",
            Self::InvalidGrant => "invalid_grant",
            Self::UnauthorizedClient => "unauthorized_client",
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::InvalidScope => "invalid_scope",
            Self::InvalidTarget => "invalid_target",
            Self::ServerError => "server_error",
        }
    }

    fn status(self) -> StatusCode {
        match self {
            Self::InvalidClient => StatusCode::UNAUTHORIZED,
            Self::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

/// The body of an OAuth error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthErrorBody {
    /// The RFC 6749 section 5.2 code, or `invalid_target` (RFC 8707).
    pub error: String,
    pub error_description: String,
}

/// An OAuth error response.
#[derive(Debug)]
pub struct OAuthError {
    pub code: OAuthErrorCode,
    pub description: String,
    /// The client tried HTTP Basic, so a 401 names the scheme it should
    /// retry with (RFC 6749 section 5.2).
    pub basic_challenge: bool,
}

impl OAuthError {
    pub fn new(code: OAuthErrorCode, description: impl Into<String>) -> Self {
        Self {
            code,
            description: description.into(),
            basic_challenge: false,
        }
    }

    pub fn invalid_request(description: impl Into<String>) -> Self {
        Self::new(OAuthErrorCode::InvalidRequest, description)
    }

    pub fn invalid_client(description: impl Into<String>) -> Self {
        Self::new(OAuthErrorCode::InvalidClient, description)
    }

    pub fn invalid_grant(description: impl Into<String>) -> Self {
        Self::new(OAuthErrorCode::InvalidGrant, description)
    }

    pub fn invalid_target(description: impl Into<String>) -> Self {
        Self::new(OAuthErrorCode::InvalidTarget, description)
    }

    /// Mark the error as answering a client that tried HTTP Basic.
    pub fn with_basic_challenge(mut self, tried_basic: bool) -> Self {
        self.basic_challenge = tried_basic;
        self
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let status = self.code.status();
        let mut response = (
            status,
            Json(OAuthErrorBody {
                error: self.code.as_str().into(),
                error_description: self.description,
            }),
        )
            .into_response();
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
        if status == StatusCode::UNAUTHORIZED && self.basic_challenge {
            headers.insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"maidan\""),
            );
        }
        response
    }
}

/// A store or internal failure: logged, and answered as `server_error`
/// without its detail.
impl From<maidan_store::StoreError> for OAuthError {
    fn from(err: maidan_store::StoreError) -> Self {
        tracing::error!(error = %err, "oauth endpoint store failure");
        Self::new(
            OAuthErrorCode::ServerError,
            "the request could not be completed",
        )
    }
}

impl From<ApiError> for OAuthError {
    fn from(err: ApiError) -> Self {
        tracing::error!(error = ?err, "oauth endpoint failure");
        Self::new(
            OAuthErrorCode::ServerError,
            "the request could not be completed",
        )
    }
}

/// An `application/x-www-form-urlencoded` body whose rejection is an OAuth
/// error. A body over the size limit stays the 413 problem every route
/// answers; anything else that cannot be read is `invalid_request`,
/// including a repeated parameter (RFC 6749 section 3.2).
pub struct OAuthForm<T>(pub T);

impl<T> crate::routing::Checked for OAuthForm<T> {}

impl<T, S> axum::extract::FromRequest<S> for OAuthForm<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Response> {
        axum::extract::Form::<T>::from_request(req, state)
            .await
            .map(|axum::extract::Form(value)| Self(value))
            .map_err(|e| {
                if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::PayloadTooLarge(e.body_text()).into_response()
                } else {
                    OAuthError::invalid_request(e.body_text()).into_response()
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_code_answers_its_rfc_6749_status() {
        for (code, status) in [
            (OAuthErrorCode::InvalidRequest, 400),
            (OAuthErrorCode::InvalidClient, 401),
            (OAuthErrorCode::InvalidGrant, 400),
            (OAuthErrorCode::UnauthorizedClient, 400),
            (OAuthErrorCode::UnsupportedGrantType, 400),
            (OAuthErrorCode::InvalidScope, 400),
            (OAuthErrorCode::InvalidTarget, 400),
            (OAuthErrorCode::ServerError, 500),
        ] {
            let response = OAuthError::new(code, "x").into_response();
            assert_eq!(response.status().as_u16(), status, "{}", code.as_str());
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-store"
            );
        }
    }

    #[test]
    fn only_an_invalid_client_that_tried_basic_gets_a_basic_challenge() {
        let tried = OAuthError::invalid_client("x")
            .with_basic_challenge(true)
            .into_response();
        assert_eq!(
            tried.headers().get(header::WWW_AUTHENTICATE).unwrap(),
            "Basic realm=\"maidan\""
        );
        let posted = OAuthError::invalid_client("x").into_response();
        assert!(posted.headers().get(header::WWW_AUTHENTICATE).is_none());
        let grant = OAuthError::invalid_grant("x")
            .with_basic_challenge(true)
            .into_response();
        assert!(grant.headers().get(header::WWW_AUTHENTICATE).is_none());
    }
}
