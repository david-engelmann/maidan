//! Protocol version negotiation (§3.6.2): every operation names the A2A
//! version it speaks, in the `A2A-Version` header (gRPC: `a2a-version`
//! metadata) or the `A2A-Version` query parameter. A request that names none
//! speaks 0.3, which this server does not serve.

use axum::http::HeaderMap;
use maidan_a2a::{
    is_supported_version, A2aError, A2aErrorKind, A2A_PROTOCOL_VERSION, A2A_VERSION_HEADER,
};

/// The version an HTTP request names: the header, else the query parameter.
pub(super) fn check(headers: &HeaderMap, query: Option<&str>) -> Result<(), A2aError> {
    let header = headers
        .get(A2A_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let param = || {
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .find(|(key, _)| key.eq_ignore_ascii_case(A2A_VERSION_HEADER))
            .map(|(_, value)| value.into_owned())
    };
    supported(header.or_else(param).as_deref())
}

/// The version a gRPC call names in its `a2a-version` metadata.
pub(crate) fn check_grpc_version(metadata: &tonic::metadata::MetadataMap) -> Result<(), A2aError> {
    let requested = metadata.get("a2a-version").and_then(|v| v.to_str().ok());
    supported(requested)
}

fn supported(requested: Option<&str>) -> Result<(), A2aError> {
    let requested = requested
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("0.3");
    if is_supported_version(requested) {
        return Ok(());
    }
    Err(A2aError::new(
        A2aErrorKind::VersionNotSupported,
        format!(
            "A2A version {requested} is not supported; this agent speaks {A2A_PROTOCOL_VERSION}"
        ),
    )
    .with("requestedVersion", requested)
    .with("supportedVersions", A2A_PROTOCOL_VERSION))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(version: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(A2A_VERSION_HEADER, version.parse().unwrap());
        map
    }

    #[test]
    fn header_or_query_names_the_version() {
        assert!(check(&headers("1.0"), None).is_ok());
        assert!(check(&headers("1.0.2"), None).is_ok());
        assert!(check(&HeaderMap::new(), Some("x=1&A2A-Version=1.0")).is_ok());
        // The header wins over the query parameter.
        assert!(check(&headers("0.3"), Some("A2A-Version=1.0")).is_err());
    }

    #[test]
    fn a_missing_or_foreign_version_is_refused() {
        for (headers, query) in [
            (HeaderMap::new(), None),
            (headers(""), None),
            (headers("0.3"), None),
            (headers("2.0"), None),
            (HeaderMap::new(), Some("A2A-Version=1.1")),
        ] {
            let err = check(&headers, query).unwrap_err();
            assert_eq!(err.kind, A2aErrorKind::VersionNotSupported);
        }
        let err = check(&HeaderMap::new(), None).unwrap_err();
        assert_eq!(err.metadata["requestedVersion"], "0.3");
    }
}
