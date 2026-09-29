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
    }
    let client = builder.build().map_err(|error| error.to_string())?;
    Ok((client, target.url))
}
