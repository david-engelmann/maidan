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
    // RFC 9637 `3fff::/20`: the first hextet is `3fff` and the next four bits are 0.
    let documentation = segments[0] == 0x3fff && (segments[1] & 0xf000) == 0;
    let amt = segments[0] == 0x2001 && segments[1] == 3;
    let as112 = segments[0] == 0x2001 && segments[1] == 4 && segments[2] == 0x0112;
    let drone = segments[0] == 0x2001 && (segments[1] & 0xfff0) == 0x0030;
    let direct_as112 = segments[0] == 0x2620 && segments[1] == 0x004f && segments[2] == 0x8000;
    (segments[0] & 0xe000) == 0x2000
        && !(segments[0] == 0x2001
            && (segments[1] == 0
                || segments[1] == 2
                || segments[1] == 0x0db8
                || (segments[1] & 0xfff0) == 0x0010
                || (segments[1] & 0xfff0) == 0x0020))
        && segments[0] != 0x2002
        && !documentation
        && !amt
        && !as112
        && !drone
        && !direct_as112
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
                || host.to_ascii_lowercase().ends_with(".localhost")
                || is_internal_name(host) =>
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

/// The environment variable naming the peers that may live on a private
/// network.
pub const PRIVATE_EGRESS_HOSTS_ENV: &str = "MAIDAN_PRIVATE_EGRESS_HOSTS";

/// Parses a comma-separated list of hostnames or address literals. Entries
/// are trimmed and lowercased; an IPv6 literal may be bracketed. An entry that
/// carries a scheme, a port, a path or a wildcard is dropped, because it
/// would never match a URL's host and a partial match is not allowed.
pub fn parse_private_egress_hosts(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|entry| entry.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .map(str::to_string)
                .unwrap_or(entry)
        })
        .filter(|entry| entry.parse::<IpAddr>().is_ok() || is_bare_name(entry))
        .collect()
}

/// The peers this instance may reach on a private network
/// (`MAIDAN_PRIVATE_EGRESS_HOSTS`). Unlike `MAIDAN_ALLOW_PRIVATE_EGRESS`, which
/// opens the whole guard in development, this names each host, holds in
/// production, and never opens link-local addresses (cloud metadata).
pub fn private_egress_hosts() -> Vec<String> {
    std::env::var(PRIVATE_EGRESS_HOSTS_ENV)
        .map(|raw| parse_private_egress_hosts(&raw))
        .unwrap_or_default()
}

/// The environment variable naming the `.internal` hosts this instance may
/// send to: names its own platform answers, never the public internet.
pub const INTERNAL_EGRESS_HOSTS_ENV: &str = "MAIDAN_INTERNAL_EGRESS_HOSTS";

const INTERNAL_SUFFIX: &str = ".internal";

/// Whether `host` is a name under `.internal`, the top-level domain ICANN
/// reserved for private use: no public resolver answers for it, so all such a
/// name can reach is what the instance's own network maps it to.
fn is_internal_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.len() > INTERNAL_SUFFIX.len() && host.ends_with(INTERNAL_SUFFIX)
}

fn is_bare_name(entry: &str) -> bool {
    entry.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Parses a comma-separated list of `.internal` hostnames. Entries are
/// trimmed, lowercased and stripped of a trailing dot; an entry that is not a
/// bare name under `.internal` (an address, a scheme, a port, a path, a
/// wildcard, any other domain) is dropped, so the list can never open a
/// public or private address.
pub fn parse_internal_egress_hosts(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|entry| entry.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|entry| is_internal_name(entry) && is_bare_name(entry))
        .collect()
}

/// The `.internal` hosts this instance may send to
/// (`MAIDAN_INTERNAL_EGRESS_HOSTS`). On Cloudflare Containers the Worker maps
/// each to a service binding (`MAIDAN_INTERNAL_HOSTS`), so a webhook to
/// `http://seats.internal/wake` reaches our own Worker without crossing the
/// public internet. Exact names only; an unlisted `.internal` name is refused.
pub fn internal_egress_hosts() -> Vec<String> {
    std::env::var(INTERNAL_EGRESS_HOSTS_ENV)
        .map(|raw| parse_internal_egress_hosts(&raw))
        .unwrap_or_default()
}

fn url_host_key(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(host) => Some(host.trim_end_matches('.').to_ascii_lowercase()),
        Host::Ipv4(ip) => Some(ip.to_string()),
        Host::Ipv6(ip) => Some(ip.to_string()),
    }
}

fn host_is_listed(url: &Url, hosts: &[String]) -> bool {
    url_host_key(url).is_some_and(|key| {
        hosts.iter().any(|entry| {
            entry.trim_end_matches('.') == key
                || entry
                    .parse::<IpAddr>()
                    .ok()
                    .zip(key.parse::<IpAddr>().ok())
                    .is_some_and(|(a, b)| a == b)
        })
    })
}

/// Whether `ip` is link-local (IPv4 169.254/16, IPv6 fe80::/10, or an IPv4
/// mapped link-local), which a listed private peer still may not resolve to.
fn is_link_local(ip: IpAddr) -> bool {
    match peer_address(ip) {
        IpAddr::V4(ip) => ip.is_link_local(),
        IpAddr::V6(ip) => (ip.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// IPv4 mapped and NAT64 addresses as the IPv4 they carry, otherwise `ip`.
fn peer_address(ip: IpAddr) -> IpAddr {
    let IpAddr::V6(v6) = ip else {
        return ip;
    };
    if let Some(mapped) = v6.to_ipv4_mapped() {
        return IpAddr::V4(mapped);
    }
    let segments = v6.segments();
    let well_known = segments[0] == 0x0064
        && segments[1] == 0xff9b
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0;
    let local = segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 1;
    if well_known || local {
        let embedded = ((segments[6] as u32) << 16) | u32::from(segments[7]);
        return IpAddr::V4(Ipv4Addr::from(embedded));
    }
    ip
}

/// Addresses a listed peer may not be: unspecified, multicast, broadcast, and
/// link-local, including those embedded in IPv4-mapped or NAT64 form.
/// Loopback stays allowed: a listed host may resolve to it.
fn is_disallowed_peer_ip(ip: IpAddr) -> bool {
    match peer_address(ip) {
        IpAddr::V4(ip) => {
            ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() || ip.is_link_local()
        }
        IpAddr::V6(ip) => ip.is_unspecified() || ip.is_multicast() || is_link_local(IpAddr::V6(ip)),
    }
}

/// How far the address guard opens for one target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivateAccess {
    /// Public addresses only.
    None,
    /// A listed peer: private addresses, but never link-local.
    ListedPeer,
    /// A listed `.internal` host. The platform's network answers its name
    /// with a placeholder address that routes to a service binding, so the
    /// address is not judged; the name was.
    Internal,
    /// The development escape hatch: anything.
    Everything,
}

fn validate_scheme_and_credentials(raw: &str) -> Result<Url, EgressTargetError> {
    if raw.len() > MAX_EGRESS_URL_LEN || raw.trim() != raw {
        return Err(EgressTargetError::InvalidUrl);
    }
    let url = Url::parse(raw).map_err(|_| EgressTargetError::InvalidUrl)?;
    if url.as_str().len() > MAX_EGRESS_URL_LEN {
        return Err(EgressTargetError::InvalidUrl);
    }
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(EgressTargetError::InvalidUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(EgressTargetError::Credentials);
    }
    Ok(url)
}

fn validate_with(
    raw: &str,
    hosts: &[String],
    internal: &[String],
) -> Result<(Url, PrivateAccess), EgressTargetError> {
    match parse_egress_target(raw) {
        Ok(url) => {
            let access = if host_is_listed(&url, hosts) {
                PrivateAccess::ListedPeer
            } else {
                PrivateAccess::None
            };
            Ok((url, access))
        }
        Err(EgressTargetError::NonPublic) => {
            let url = validate_scheme_and_credentials(raw)?;
            let literal_disallowed = match url.host() {
                Some(Host::Ipv4(ip)) => is_disallowed_peer_ip(IpAddr::V4(ip)),
                Some(Host::Ipv6(ip)) => is_disallowed_peer_ip(IpAddr::V6(ip)),
                _ => false,
            };
            let internal_name =
                matches!(url.host(), Some(Host::Domain(host)) if is_internal_name(host));
            if internal_name && host_is_listed(&url, internal) {
                Ok((url, PrivateAccess::Internal))
            } else if host_is_listed(&url, hosts) && !literal_disallowed {
                Ok((url, PrivateAccess::ListedPeer))
            } else {
                Err(EgressTargetError::NonPublic)
            }
        }
        Err(other) => Err(other),
    }
}

pub fn validate_egress_target(raw: &str) -> Result<Url, EgressTargetError> {
    if !private_egress_explicitly_allowed() {
        return validate_with(raw, &private_egress_hosts(), &internal_egress_hosts())
            .map(|(url, _)| url);
    }
    validate_scheme_and_credentials(raw)
}

/// Resolves `raw`, giving the DNS lookup [`EGRESS_RESOLUTION_TIMEOUT`]. A
/// stalled resolver is [`EgressTargetError::ResolutionTimedOut`], not a hang.
pub async fn resolve_egress_target(raw: &str) -> Result<ResolvedEgressTarget, EgressTargetError> {
    let (url, access) = if private_egress_explicitly_allowed() {
        (validate_egress_target(raw)?, PrivateAccess::Everything)
    } else {
        validate_with(raw, &private_egress_hosts(), &internal_egress_hosts())?
    };
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
    ensure_allowed_addresses(&addresses, access)?;
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

fn ensure_allowed_addresses(
    addresses: &[SocketAddr],
    access: PrivateAccess,
) -> Result<(), EgressTargetError> {
    let refused = match access {
        PrivateAccess::Everything | PrivateAccess::Internal => false,
        PrivateAccess::ListedPeer => addresses
            .iter()
            .any(|addr| is_disallowed_peer_ip(addr.ip())),
        PrivateAccess::None => addresses.iter().any(|addr| !is_public_ip(addr.ip())),
    };
    if refused {
        return Err(EgressTargetError::NonPublic);
    }
    Ok(())
}

#[cfg(test)]
fn ensure_public_addresses(
    addresses: &[SocketAddr],
    allow_private: bool,
) -> Result<(), EgressTargetError> {
    let access = if allow_private {
        PrivateAccess::Everything
    } else {
        PrivateAccess::None
    };
    ensure_allowed_addresses(addresses, access)
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
            "http://seats.internal/x",
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

    fn hosts(raw: &str) -> Vec<String> {
        parse_private_egress_hosts(raw)
    }

    fn validate_with_peers(
        raw: &str,
        peers: &[String],
    ) -> Result<(Url, PrivateAccess), EgressTargetError> {
        validate_with(raw, peers, &[])
    }

    fn access(raw: &str, internal: &[String]) -> Result<PrivateAccess, EgressTargetError> {
        validate_with(raw, &[], internal).map(|(_, access)| access)
    }

    #[test]
    fn the_internal_list_keeps_only_bare_names_under_internal() {
        assert_eq!(
            parse_internal_egress_hosts(
                " Seats.Internal. , control.internal, internal, .internal, 10.0.0.5, \
                 http://a.internal, a.internal:80, *.internal, a..internal, example.com, \
                 internal.example.com, a_b.internal, ,"
            ),
            vec!["seats.internal", "control.internal"]
        );
    }

    #[test]
    fn a_listed_internal_host_is_allowed_over_plain_http() {
        let internal = parse_internal_egress_hosts("seats.internal");
        for raw in [
            "http://seats.internal/wake",
            "http://SEATS.internal./wake?thread=1",
            "https://seats.internal/wake",
        ] {
            assert_eq!(
                access(raw, &internal).ok(),
                Some(PrivateAccess::Internal),
                "{raw}"
            );
        }
        // The placeholder address the platform answers with is not judged.
        let placeholder: [SocketAddr; 1] = ["169.254.0.7:80".parse().expect("placeholder")];
        assert!(ensure_allowed_addresses(&placeholder, PrivateAccess::Internal).is_ok());
        assert!(matches!(
            access("http://u:p@seats.internal/wake", &internal),
            Err(EgressTargetError::Credentials)
        ));
    }

    #[test]
    fn nothing_but_a_listed_internal_name_opens() {
        let internal = parse_internal_egress_hosts("seats.internal");
        // An unlisted `.internal` name, even one that ends like a listed one.
        for raw in [
            "http://control.internal/mcp",
            "http://evil.seats.internal/wake",
            "http://seats.internal.example.com/wake",
        ] {
            let result = access(raw, &internal);
            if raw.ends_with(".com/wake") {
                // Not under `.internal` at all: a public name, public answers only.
                assert_eq!(result.ok(), Some(PrivateAccess::None), "{raw}");
            } else {
                assert!(
                    matches!(result, Err(EgressTargetError::NonPublic)),
                    "opened {raw}"
                );
            }
        }
        // With no list, every `.internal` name is refused, as `.localhost` is.
        assert!(matches!(
            access("http://seats.internal/wake", &[]),
            Err(EgressTargetError::NonPublic)
        ));
        assert!(matches!(
            parse_egress_target("http://seats.internal/wake"),
            Err(EgressTargetError::NonPublic)
        ));
        // Plain http to a public host keeps today's rule: allowed, but with
        // public-only addresses, never the internal pass.
        assert_eq!(
            access("http://example.com/hook", &internal).ok(),
            Some(PrivateAccess::None)
        );
        // A private literal or loopback is not opened by the internal list.
        for raw in [
            "http://10.0.0.5/x",
            "http://127.0.0.1/x",
            "http://localhost/x",
        ] {
            assert!(
                matches!(access(raw, &internal), Err(EgressTargetError::NonPublic)),
                "{raw}"
            );
        }
        // The peer list does not hand an internal name the unchecked pass.
        let peers = hosts("seats.internal");
        assert_eq!(
            validate_with_peers("http://seats.internal/wake", &peers)
                .map(|(_, a)| a)
                .ok(),
            Some(PrivateAccess::ListedPeer)
        );
    }

    /// Webhook targets are per workspace, the list is per instance: every
    /// workspace admin is held to the same exact names, and one workspace's
    /// target never widens what another may register.
    #[test]
    fn every_workspace_is_held_to_the_same_internal_list() {
        let internal = parse_internal_egress_hosts("seats.internal");
        // What two workspaces' admins each try to register, in turn.
        let workspace_a = [
            ("http://seats.internal/wake", true),
            ("http://admin.internal/x", false),
        ];
        let workspace_b = [
            ("http://seats.internal/wake", true),
            ("http://control.internal/x", false),
            ("http://admin.internal/x", false),
        ];
        for (target, allowed) in workspace_a.into_iter().chain(workspace_b) {
            let result = access(target, &internal);
            if allowed {
                assert_eq!(result.ok(), Some(PrivateAccess::Internal), "{target}");
            } else {
                assert!(
                    matches!(result, Err(EgressTargetError::NonPublic)),
                    "{target}"
                );
            }
        }
    }

    #[test]
    fn the_peer_list_keeps_only_bare_hosts_and_literals() {
        assert_eq!(
            hosts(" Peer.Internal , [fd00::7], 10.0.0.5, https://x.y, a.b:80, *.corp, ,"),
            vec!["peer.internal", "fd00::7", "10.0.0.5"]
        );
    }

    #[test]
    fn a_listed_peer_may_be_private_and_nothing_else_opens() {
        let list = hosts("peer.internal,10.0.0.5,fd00::7");
        assert_eq!(
            validate_with_peers("https://10.0.0.5/a2a", &list)
                .map(|(_, a)| a)
                .ok(),
            Some(PrivateAccess::ListedPeer)
        );
        assert_eq!(
            validate_with_peers("http://[fd00::7]:9000/x", &list)
                .map(|(_, a)| a)
                .ok(),
            Some(PrivateAccess::ListedPeer)
        );
        assert_eq!(
            validate_with_peers("https://PEER.internal./x", &list)
                .map(|(_, a)| a)
                .ok(),
            Some(PrivateAccess::ListedPeer)
        );
        for refused in [
            "https://10.0.0.6/a2a",
            "http://127.0.0.1/x",
            "http://localhost/x",
            "https://other.internal.peer.internal.evil/x",
        ] {
            let result = validate_with_peers(refused, &list);
            if refused.contains("evil") {
                // A public-looking name is accepted at parse time but gets no
                // private access, so its DNS answer must be public.
                assert_eq!(result.map(|(_, a)| a).ok(), Some(PrivateAccess::None));
            } else {
                assert!(
                    matches!(result, Err(EgressTargetError::NonPublic)),
                    "opened {refused}"
                );
            }
        }
        assert!(matches!(
            validate_with_peers("https://u:p@10.0.0.5/x", &list),
            Err(EgressTargetError::Credentials)
        ));
    }

    #[test]
    fn a_listed_link_local_literal_stays_closed() {
        let list = hosts("169.254.169.254,fe80::1");
        assert!(matches!(
            validate_with_peers("http://169.254.169.254/latest/meta-data", &list),
            Err(EgressTargetError::NonPublic)
        ));
        assert!(matches!(
            validate_with_peers("http://[fe80::1]/x", &list),
            Err(EgressTargetError::NonPublic)
        ));
    }

    #[test]
    fn a_listed_peer_resolving_to_link_local_is_refused() {
        let private: [SocketAddr; 1] = ["10.0.0.5:443".parse().expect("private address")];
        let metadata: [SocketAddr; 2] = [
            "10.0.0.5:443".parse().expect("private address"),
            "169.254.169.254:80".parse().expect("metadata address"),
        ];
        let mapped: [SocketAddr; 1] = ["[::ffff:169.254.1.1]:80".parse().expect("mapped")];
        assert!(ensure_allowed_addresses(&private, PrivateAccess::ListedPeer).is_ok());
        assert!(ensure_allowed_addresses(&private, PrivateAccess::None).is_err());
        assert!(ensure_allowed_addresses(&metadata, PrivateAccess::ListedPeer).is_err());
        assert!(ensure_allowed_addresses(&mapped, PrivateAccess::ListedPeer).is_err());
        assert!(ensure_allowed_addresses(&metadata, PrivateAccess::Everything).is_ok());
        let loopback: [SocketAddr; 1] = ["127.0.0.1:80".parse().expect("loopback")];
        assert!(ensure_allowed_addresses(&loopback, PrivateAccess::ListedPeer).is_ok());
        let unspecified: [SocketAddr; 1] = ["0.0.0.0:80".parse().expect("unspecified")];
        let broadcast: [SocketAddr; 1] = ["255.255.255.255:80".parse().expect("broadcast")];
        let multicast: [SocketAddr; 1] = ["224.0.0.1:80".parse().expect("multicast")];
        let nat64_meta: [SocketAddr; 1] =
            ["[64:ff9b::a9fe:a9fe]:80".parse().expect("nat64 metadata")];
        for refused in [unspecified, broadcast, multicast, nat64_meta] {
            assert!(ensure_allowed_addresses(&refused, PrivateAccess::ListedPeer).is_err());
        }
    }

    #[test]
    fn special_purpose_ipv6_is_not_a_public_peer() {
        for raw in [
            "http://[3fff::1]/x",
            "http://[2001:3::1]/x",
            "http://[2001:4:112::1]/x",
            "http://[2001:30::1]/x",
            "http://[2620:4f:8000::1]/x",
        ] {
            assert!(
                matches!(parse_egress_target(raw), Err(EgressTargetError::NonPublic)),
                "opened {raw}"
            );
        }
    }

    #[test]
    fn malformed_peer_names_are_dropped() {
        assert_eq!(
            hosts(".., .corp, -bad.corp, a..b, peer.internal., 10.1.2.3"),
            vec!["peer.internal", "10.1.2.3"]
        );
    }

    #[test]
    fn a_listed_loopback_literal_stays_open_and_unspecified_stays_closed() {
        let list = hosts("127.0.0.1,0.0.0.0,::");
        assert_eq!(
            validate_with_peers("http://127.0.0.1/x", &list)
                .map(|(_, access)| access)
                .ok(),
            Some(PrivateAccess::ListedPeer)
        );
        assert!(matches!(
            validate_with_peers("http://0.0.0.0/x", &list),
            Err(EgressTargetError::NonPublic)
        ));
        assert!(matches!(
            validate_with_peers("http://[::]/x", &list),
            Err(EgressTargetError::NonPublic)
        ));
    }

    #[test]
    fn the_private_escape_hatch_still_bounds_the_url() {
        unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
        assert!(matches!(
            validate_egress_target(&format!(
                "http://10.0.0.5/{}",
                "a".repeat(MAX_EGRESS_URL_LEN)
            )),
            Err(EgressTargetError::InvalidUrl)
        ));
        assert!(matches!(
            validate_egress_target(" http://10.0.0.5/x"),
            Err(EgressTargetError::InvalidUrl)
        ));
        assert!(validate_egress_target("http://10.0.0.5/x").is_ok());
        unsafe { std::env::remove_var("MAIDAN_ALLOW_PRIVATE_EGRESS") };
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
