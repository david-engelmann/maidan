//! Fail-closed validation for operator-supplied HTTP egress targets.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use thiserror::Error;
use url::{Host, Url};

/// How long a DNS lookup for an egress target may take. The HTTP client's
/// connect and total timeouts start only after [`resolve_egress_target`]
/// returns, and the webhook poller sends one delivery at a time, so a lookup
/// that never answers used to hold every tenant's webhooks.
pub const EGRESS_RESOLUTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum EgressTargetError {
    #[error("egress url must be an absolute http or https URL")]
    InvalidUrl,
    #[error("egress url must not contain credentials")]
    Credentials,
    #[error("egress target is not a public network address")]
    NonPublic,
    #[error("egress target could not be resolved")]
    Unresolvable,
    #[error("egress target resolution timed out")]
    ResolutionTimedOut,
}

#[derive(Clone, Debug)]
pub struct ResolvedEgressTarget {
    pub url: Url,
    pub host: String,
    pub addresses: Vec<SocketAddr>,
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !matches!(
        (a, b, c),
        (0, _, _)
            | (10, _, _)
            | (100, 64..=127, _)
            | (127, _, _)
            | (169, 254, _)
            | (172, 16..=31, _)
            | (192, 0, 0)
            | (192, 0, 2)
            | (192, 88, 99)
            | (192, 168, _)
            | (198, 18..=19, _)
            | (198, 51, 100)
            | (203, 0, 113)
            | (224..=255, _, _)
    )
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = ip.segments();
    // Public unicast allocations are within 2000::/3. Stay deliberately
    // conservative and exclude special-use tunnels and documentation blocks
    // inside that space as well.
    (segments[0] & 0xe000) == 0x2000
        && !(segments[0] == 0x2001
            && (segments[1] == 0
                || segments[1] == 2
                || segments[1] == 0x0db8
                || (segments[1] & 0xfff0) == 0x0010
                || (segments[1] & 0xfff0) == 0x0020))
        && segments[0] != 0x2002
}

const MAX_EGRESS_URL_LEN: usize = 2048;

pub fn parse_egress_target(raw: &str) -> Result<Url, EgressTargetError> {
    if raw.len() > MAX_EGRESS_URL_LEN || raw.trim() != raw {
        return Err(EgressTargetError::InvalidUrl);
    }
    let url = Url::parse(raw).map_err(|_| EgressTargetError::InvalidUrl)?;
    // Parsing percent-encodes the path and punycodes the host, so the URL
    // this returns can be longer than what was sent. Bounding only the input
    // accepted URLs whose own form the guard then refused (found by the
    // `egress_target` fuzz target).
    if url.as_str().len() > MAX_EGRESS_URL_LEN {
        return Err(EgressTargetError::InvalidUrl);
    }
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(EgressTargetError::InvalidUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(EgressTargetError::Credentials);
    }
    match url.host() {
        Some(Host::Domain(host))
            if host.eq_ignore_ascii_case("localhost")
                || host.to_ascii_lowercase().ends_with(".localhost") =>
        {
            Err(EgressTargetError::NonPublic)
        }
        Some(Host::Ipv4(ip)) if !is_public_ipv4(ip) => Err(EgressTargetError::NonPublic),
        Some(Host::Ipv6(ip)) if !is_public_ipv6(ip) => Err(EgressTargetError::NonPublic),
        Some(_) => Ok(url),
        None => Err(EgressTargetError::InvalidUrl),
    }
}

/// Whether the development-only private-egress escape hatch is on (never in
/// production).
pub fn private_egress_explicitly_allowed() -> bool {
    std::env::var("MAIDAN_ENV").as_deref() != Ok("production")
        && std::env::var("MAIDAN_ALLOW_PRIVATE_EGRESS").as_deref() == Ok("1")
}

pub fn validate_egress_target(raw: &str) -> Result<Url, EgressTargetError> {
    if private_egress_explicitly_allowed() {
        let url = Url::parse(raw).map_err(|_| EgressTargetError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
            return Err(EgressTargetError::InvalidUrl);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(EgressTargetError::Credentials);
        }
        Ok(url)
    } else {
        parse_egress_target(raw)
    }
}

/// Resolves `raw`, giving the DNS lookup [`EGRESS_RESOLUTION_TIMEOUT`]. A
/// stalled resolver is [`EgressTargetError::ResolutionTimedOut`], not a hang.
pub async fn resolve_egress_target(raw: &str) -> Result<ResolvedEgressTarget, EgressTargetError> {
    let allow_private = private_egress_explicitly_allowed();
    let url = validate_egress_target(raw)?;
    let host = url
        .host_str()
        .ok_or(EgressTargetError::InvalidUrl)?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or(EgressTargetError::InvalidUrl)?;
    let addresses = lookup_with_deadline(
        tokio::net::lookup_host((host.as_str(), port)),
        EGRESS_RESOLUTION_TIMEOUT,
    )
    .await?;
    ensure_public_addresses(&addresses, allow_private)?;
    Ok(ResolvedEgressTarget {
        url,
        host,
        addresses,
    })
}

async fn lookup_with_deadline<F, I>(
    lookup: F,
    deadline: Duration,
) -> Result<Vec<SocketAddr>, EgressTargetError>
where
    F: Future<Output = std::io::Result<I>>,
    I: IntoIterator<Item = SocketAddr>,
{
    match tokio::time::timeout(deadline, lookup).await {
        Ok(Ok(found)) => {
            let addresses: Vec<_> = found.into_iter().collect();
            if addresses.is_empty() {
                Err(EgressTargetError::Unresolvable)
            } else {
                Ok(addresses)
            }
        }
        Ok(Err(_)) => Err(EgressTargetError::Unresolvable),
        Err(_) => Err(EgressTargetError::ResolutionTimedOut),
    }
}

fn ensure_public_addresses(
    addresses: &[SocketAddr],
    allow_private: bool,
) -> Result<(), EgressTargetError> {
    if !allow_private && addresses.iter().any(|addr| !is_public_ip(addr.ip())) {
        return Err(EgressTargetError::NonPublic);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_every_non_public_literal_family() {
        for raw in [
            "http://127.0.0.1/x",
            "http://10.1.2.3/x",
            "http://169.254.169.254/latest/meta-data",
            "http://192.168.1.2/x",
            "http://[::1]/x",
            "http://[fe80::1]/x",
            "http://[fc00::1]/x",
            "http://[64:ff9b::7f00:1]/x",
            "http://[100::1]/x",
            "http://[2001::1]/x",
            "http://[2002:7f00:1::]/x",
            "http://[4000::1]/x",
            "http://localhost/x",
            "http://api.localhost/x",
        ] {
            assert!(parse_egress_target(raw).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn accepts_public_http_targets_without_credentials() {
        assert!(parse_egress_target("https://example.com/hooks?id=1").is_ok());
        assert!(parse_egress_target("http://8.8.8.8:8080/hook").is_ok());
        assert!(matches!(
            parse_egress_target("https://user:password@example.com/hook"),
            Err(EgressTargetError::Credentials)
        ));
    }

    #[test]
    fn classifies_ipv4_mapped_ipv6_by_the_embedded_address() {
        assert!(!is_public_ip(
            "::ffff:127.0.0.1".parse().expect("mapped loopback")
        ));
        assert!(is_public_ip(
            "::ffff:8.8.8.8".parse().expect("mapped public")
        ));
    }

    #[test]
    fn a_url_that_percent_encodes_past_the_limit_is_refused() {
        // 1,220 bytes as sent; each `é` prints as `%C3%A9`, 3,620 bytes.
        let raw = format!("https://example.com/{}", "é".repeat(600));
        assert!(raw.len() <= MAX_EGRESS_URL_LEN);
        assert!(matches!(
            parse_egress_target(&raw),
            Err(EgressTargetError::InvalidUrl)
        ));

        let fits = format!("https://example.com/{}", "é".repeat(300));
        let url = parse_egress_target(&fits).expect("1,820 bytes printed");
        let again = parse_egress_target(url.as_str()).expect("its own form passes");
        assert_eq!(again, url);
    }

    #[test]
    fn rejects_a_dns_answer_set_if_any_address_is_non_public() {
        let addresses = [
            "8.8.8.8:443".parse().expect("public address"),
            "127.0.0.1:443".parse().expect("loopback address"),
        ];
        assert!(matches!(
            ensure_public_addresses(&addresses, false),
            Err(EgressTargetError::NonPublic)
        ));
        assert!(ensure_public_addresses(&addresses, true).is_ok());
    }

    #[test]
    fn the_dns_deadline_is_five_seconds() {
        assert_eq!(EGRESS_RESOLUTION_TIMEOUT, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_stalled_dns_lookup_ends_at_its_deadline() {
        let deadline = Duration::from_millis(40);
        let started = std::time::Instant::now();
        let result = lookup_with_deadline(
            std::future::pending::<std::io::Result<std::vec::IntoIter<SocketAddr>>>(),
            deadline,
        )
        .await;
        let waited = started.elapsed();
        assert!(matches!(result, Err(EgressTargetError::ResolutionTimedOut)));
        assert!(
            waited >= deadline && waited < Duration::from_secs(2),
            "ended after {waited:?}, not at the deadline"
        );
    }

    #[tokio::test]
    async fn a_failed_lookup_is_unresolvable_rather_than_a_timeout() {
        let result = lookup_with_deadline(
            std::future::ready(Err::<std::vec::IntoIter<SocketAddr>, _>(
                std::io::Error::other("no such host"),
            )),
            EGRESS_RESOLUTION_TIMEOUT,
        )
        .await;
        assert!(matches!(result, Err(EgressTargetError::Unresolvable)));
    }

    #[tokio::test]
    async fn an_empty_answer_is_unresolvable() {
        let result = lookup_with_deadline(
            std::future::ready(Ok(Vec::<SocketAddr>::new().into_iter())),
            EGRESS_RESOLUTION_TIMEOUT,
        )
        .await;
        assert!(matches!(result, Err(EgressTargetError::Unresolvable)));
    }

    #[tokio::test]
    async fn a_public_address_literal_resolves_without_a_dns_wait() {
        let started = std::time::Instant::now();
        let target = resolve_egress_target("https://1.1.1.1/hook")
            .await
            .expect("a literal needs no nameserver");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a literal waited on DNS"
        );
        assert!(target
            .addresses
            .iter()
            .any(|addr| addr.ip() == "1.1.1.1".parse::<IpAddr>().expect("address")));
    }
}
