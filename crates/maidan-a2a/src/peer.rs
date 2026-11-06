use chrono::{DateTime, Utc};
use maidan_types::{PeerId, WorkspaceId};
use serde::{Deserialize, Serialize};

use crate::error::FederationError;

/// Remote Maidan deployment registered for event replication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub id: PeerId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    pub last_synced_event_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields required to register a new peer (store assigns id/timestamps in G.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPeer {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
}

impl NewPeer {
    pub fn validate(&self) -> Result<(), FederationError> {
        validate_peer_name(&self.name)?;
        validate_base_url(&self.base_url)?;
        Ok(())
    }
}

impl Peer {
    pub fn validate(&self) -> Result<(), FederationError> {
        validate_peer_name(&self.name)?;
        validate_base_url(&self.base_url)?;
        if self.last_synced_event_id < 0 {
            return Err(FederationError::InvalidInput(
                "last_synced_event_id must be non-negative".into(),
            ));
        }
        Ok(())
    }
}

pub fn validate_peer_name(name: &str) -> Result<(), FederationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(FederationError::InvalidInput(
            "peer name must not be empty".into(),
        ));
    }
    if trimmed.len() > 128 {
        return Err(FederationError::InvalidInput(
            "peer name must be at most 128 characters".into(),
        ));
    }
    Ok(())
}

/// `base_url` must be an absolute `http` or `https` URL without a trailing
/// slash, userinfo, query, or fragment, and it must name a host.
pub fn validate_base_url(base_url: &str) -> Result<(), FederationError> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(FederationError::InvalidInput(
            "base_url must not be empty".into(),
        ));
    }
    if trimmed.ends_with('/') {
        return Err(FederationError::InvalidInput(
            "base_url must not end with '/'".into(),
        ));
    }
    if trimmed.contains(char::is_whitespace) {
        return Err(FederationError::InvalidInput(
            "base_url must not contain whitespace".into(),
        ));
    }
    // The URL parser rewrites `https:///missing-host` into a host named
    // `missing-host`. An empty authority is rejected before that rewrite.
    if raw_host(trimmed).is_none() {
        return Err(FederationError::InvalidInput(
            "base_url must name a host".into(),
        ));
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| FederationError::InvalidInput(format!("base_url is not a valid URL: {e}")))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(FederationError::InvalidInput(
            "base_url must start with http:// or https://".into(),
        ));
    }
    if url.host_str().map(str::is_empty).unwrap_or(true) {
        return Err(FederationError::InvalidInput(
            "base_url must name a host".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FederationError::InvalidInput(
            "base_url must not include userinfo".into(),
        ));
    }
    if url.query().is_some() {
        return Err(FederationError::InvalidInput(
            "base_url must not include a query".into(),
        ));
    }
    if url.fragment().is_some() {
        return Err(FederationError::InvalidInput(
            "base_url must not include a fragment".into(),
        ));
    }
    Ok(())
}

/// Host from the authority, before a parser repairs an empty one.
fn raw_host(url: &str) -> Option<&str> {
    let (_, after) = url.split_once("://")?;
    let authority = after.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = authority
        .rsplit_once('@')
        .map(|(_, hostport)| hostport)
        .unwrap_or(authority);
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split_once(']')?.0
    } else {
        hostport
            .split_once(':')
            .map(|(host, _)| host)
            .unwrap_or(hostport)
    };
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_base_url_without_trailing_slash() {
        validate_base_url("https://peer.example.com").expect("valid url");
    }

    #[test]
    fn rejects_trailing_slash_on_base_url() {
        let err = validate_base_url("https://peer.example.com/").unwrap_err();
        assert!(matches!(err, FederationError::InvalidInput(_)));
    }

    #[test]
    fn rejects_relative_base_url() {
        validate_base_url("/relative").expect_err("relative");
    }

    #[test]
    fn base_url_rejects_a_missing_host_userinfo_query_fragment_and_a_bad_port() {
        for url in [
            "https:///missing-host",
            "https://:8080",
            "https://user:pass@peer.example",
            "https://peer.example?x=1",
            "https://peer.example#frag",
            "https://peer.example:99999",
        ] {
            validate_base_url(url).expect_err(url);
        }
    }

    #[test]
    fn base_url_accepts_a_port_a_path_prefix_and_ipv6() {
        validate_base_url("http://peer.example:8080/prefix").expect("prefix");
        validate_base_url("https://[::1]:8443").expect("ipv6");
        validate_base_url("https://[2001:db8::1]").expect("public ipv6");
    }

    #[test]
    fn new_peer_validate_requires_name_and_url() {
        let peer = NewPeer {
            workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
            name: "  ".into(),
            base_url: "https://a.example".into(),
        };
        peer.validate().expect_err("empty name");
    }

    #[test]
    fn peer_record_roundtrips_json() {
        let now = Utc::now();
        let peer = Peer {
            id: PeerId(uuid::Uuid::new_v4()),
            workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
            name: "east-region".into(),
            base_url: "https://east.maidan.example".into(),
            enabled: true,
            last_synced_event_id: 42,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&peer).expect("serialize");
        let parsed: Peer = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, peer);
        parsed.validate().expect("valid after roundtrip");
    }
}
