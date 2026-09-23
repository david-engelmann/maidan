//! HTTP client construction for untrusted, operator-supplied egress URLs.

use reqwest::{redirect::Policy, Client, Url};

pub async fn client_for(raw: &str) -> Result<(Client, Url), String> {
    let target = maidan_auth::resolve_egress_target(raw)
        .await
        .map_err(|error| error.to_string())?;
    let mut builder = Client::builder().redirect(Policy::none());
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
