//! Reusable RFC 9457 error responses (`#/components/responses/*`).
//!
//! Every error the API returns is an `application/problem+json`
//! [`ProblemDetails`](crate::error::ProblemDetails) body. An operation names the
//! statuses its handler can produce with `(status = 404, response = NotFound)`;
//! the two a middleware layer adds to every operation it wraps (401 from the
//! authentication layers, 429 from the rate limiter and per-token quotas) are
//! attached by [`MiddlewareResponses`](super::MiddlewareResponses) instead, so a
//! new route cannot forget them.

use utoipa::openapi::header::HeaderBuilder;
use utoipa::openapi::response::{Response, ResponseBuilder};
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::openapi::{Content, Ref, RefOr};
use utoipa::ToResponse;

pub(crate) const PROBLEM_JSON: &str = "application/problem+json";

fn problem(description: &str) -> ResponseBuilder {
    ResponseBuilder::new().description(description).content(
        PROBLEM_JSON,
        Content::new(Some(Ref::from_schema_name("ProblemDetails"))),
    )
}

macro_rules! problem_response {
    ($(#[$doc:meta])* $name:ident, $description:literal) => {
        $(#[$doc])*
        pub struct $name;

        impl<'r> ToResponse<'r> for $name {
            fn response() -> (&'r str, RefOr<Response>) {
                (stringify!($name), problem($description).build().into())
            }
        }
    };
}

problem_response!(
    /// 400: the handler rejected the request body or parameters.
    BadRequest,
    "The request body or parameters are malformed or fail validation."
);
problem_response!(
    /// 401: added by [`MiddlewareResponses`](super::MiddlewareResponses).
    Unauthorized,
    "The credential is missing, invalid, expired, or revoked."
);
problem_response!(
    /// 403: a capability or access check refused the caller.
    Forbidden,
    "The caller is authenticated but lacks the capability or access this needs."
);
problem_response!(
    /// 404: the resource does not exist or is not visible to the caller.
    NotFound,
    "The resource does not exist, or the caller cannot see it."
);
problem_response!(
    /// 409: the resource's current state refuses the request.
    Conflict,
    "The request conflicts with the resource's current state."
);
problem_response!(
    /// 413: the JSON body exceeds `MAIDAN_MAX_BODY_BYTES`.
    PayloadTooLarge,
    "The request body exceeds the server's body-size limit (`MAIDAN_MAX_BODY_BYTES`)."
);

/// 429: added by [`MiddlewareResponses`](super::MiddlewareResponses).
pub struct TooManyRequests;

impl<'r> ToResponse<'r> for TooManyRequests {
    fn response() -> (&'r str, RefOr<Response>) {
        let retry_after = HeaderBuilder::new()
            .schema(
                ObjectBuilder::new()
                    .schema_type(Type::Integer)
                    .minimum(Some(1)),
            )
            .description(Some(
                "Seconds until the rate-limit window resets. Sent by the per-client and \
                 per-workspace rate limits, not by a per-token capability quota.",
            ))
            .build();
        (
            "TooManyRequests",
            problem("A rate limit or a per-token capability quota is exhausted.")
                .header("Retry-After", retry_after)
                .build()
                .into(),
        )
    }
}
