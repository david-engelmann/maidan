//! HTTP client construction for untrusted, operator-supplied egress URLs.

use std::time::Duration;

use reqwest::{redirect::Policy, Client, ClientBuilder, Url};

/// How long an outbound request may take to connect.
pub const EGRESS_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an outbound request may take in all, unless its caller sets a
/// shorter one. The webhook poller sends one delivery at a time, so a receiver
/// that accepts the connection and never answers held up every tenant's
/// webhooks until this bound existed.
pub const EGRESS_TIMEOUT: Duration = Duration::from_secs(10);

/// A client builder with the egress timeouts, for every outbound HTTP call:
/// operator-supplied URLs through [`client_for`], and the Slack and GitHub
/// APIs directly.
pub fn bounded() -> ClientBuilder {
    Client::builder()
        .connect_timeout(EGRESS_CONNECT_TIMEOUT)
        .timeout(EGRESS_TIMEOUT)
}

/// A no-redirect client for an operator-supplied URL. Name resolution gives
/// up after [`maidan_auth::EGRESS_RESOLUTION_TIMEOUT`], before this client
/// exists; connecting then gives up after [`EGRESS_CONNECT_TIMEOUT`], and the
/// whole request after [`EGRESS_TIMEOUT`].
///
/// When the URL's host string differs from the normalized [`maidan_auth::EgressTarget::host`]
/// (for example a trailing-dot FQDN where the target stripped `.`), both names
/// are pinned in the DNS override map so `reqwest` cannot fall back to a second
/// unvalidated lookup.
pub async fn client_for(raw: &str) -> Result<(Client, Url), String> {
    let target = maidan_auth::resolve_egress_target(raw)
        .await
        .map_err(|error| error.to_string())?;
    let mut builder = bounded().redirect(Policy::none());
    if target
        .url
        .host()
        .is_some_and(|host| matches!(host, url::Host::Domain(_)))
    {
        builder = builder.resolve_to_addrs(&target.host, &target.addresses);
        if let Some(url_host) = target.url.host_str() {
            let url_host_norm = url_host.trim_end_matches('.').to_ascii_lowercase();
            if url_host != target.host && url_host_norm == target.host {
                // Trailing-dot (or case) form present in the URL but stripped in
                // `target.host` — pin both so the client never misses the map.
                builder = builder.resolve_to_addrs(url_host, &target.addresses);
            } else if url_host != target.host {
                builder = builder.resolve_to_addrs(url_host, &target.addresses);
            }
        }
    }
    let client = builder.build().map_err(|error| error.to_string())?;
    Ok((client, target.url))
}
