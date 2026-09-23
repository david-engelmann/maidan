//! Fail-closed validation for operator-supplied HTTP egress targets.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use thiserror::Error;
use url::{Host, Url};

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

pub fn parse_egress_target(raw: &str) -> Result<Url, EgressTargetError> {
    if raw.len() > 2048 || raw.trim() != raw {
        return Err(EgressTargetError::InvalidUrl);
    }
    let url = Url::parse(raw).map_err(|_| EgressTargetError::InvalidUrl)?;
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

fn private_egress_explicitly_allowed() -> bool {
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
    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| EgressTargetError::Unresolvable)?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(EgressTargetError::Unresolvable);
    }
    ensure_public_addresses(&addresses, allow_private)?;
    Ok(ResolvedEgressTarget {
        url,
        host,
        addresses,
    })
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
}
