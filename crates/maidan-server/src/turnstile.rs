//! Cloudflare Turnstile on the sign-up surface. With
//! `MAIDAN_TURNSTILE_SECRET` set, a waitlist join and a sign-up start must
//! carry a widget token that Cloudflare's siteverify accepts for this
//! instance's hostname and the step's action. Unset, nothing here runs: no
//! check, no widget, and no change to any page's CSP.
//!
//! The check fails closed. A missing token, a token Cloudflare rejects
//! (including one already spent), a token minted for another hostname or
//! step, and a siteverify call that cannot be made or answered all refuse
//! the request. The secret goes only into the siteverify form body; it is
//! never logged and never shown by `Debug`.

use std::fmt;

use serde::Deserialize;

use crate::config::ConfigError;
use crate::error::ApiError;

/// Cloudflare's verification endpoint.
pub const SITEVERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
/// The origin the widget script and its frame load from, for the CSP.
pub const CHALLENGE_ORIGIN: &str = "https://challenges.cloudflare.com";
/// Longest token Cloudflare issues; anything longer is not one of theirs.
const MAX_TOKEN_CHARS: usize = 2048;

/// The step a token was minted for. The widget renders with this action and
/// the server requires it back, so a token solved for the waitlist does not
/// start a sign-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Waitlist,
    Signup,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Waitlist => "waitlist",
            Action::Signup => "signup",
        }
    }
}

/// The operator's secret key. Its `Debug` is redacted.
#[derive(Clone)]
pub struct TurnstileSecret(String);

impl TurnstileSecret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for TurnstileSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TurnstileSecret(<redacted>)")
    }
}

#[derive(Clone, Debug)]
pub struct TurnstileSettings {
    /// The public site key the widget renders with.
    pub site_key: String,
    pub secret: TurnstileSecret,
    /// The hostname a token must have been solved on.
    pub hostname: String,
    /// [`SITEVERIFY_URL`] in a deployment; a test points it at a mock.
    pub siteverify_url: String,
}

fn env_value(name: &'static str) -> Result<Option<String>, ConfigError> {
    match std::env::var(name) {
        Ok(v) if v.trim().is_empty() => Ok(None),
        Ok(v) => Ok(Some(v.trim().to_string())),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(ConfigError::Invalid(name, "is not valid UTF-8".into()))
        }
    }
}

/// The host part of an origin such as `https://maidan.world`.
fn origin_host(origin: &str) -> Option<String> {
    let url = url::Url::parse(origin).ok()?;
    url.host_str().map(str::to_ascii_lowercase)
}

impl TurnstileSettings {
    /// Off (`None`) unless `MAIDAN_TURNSTILE_SECRET` is set. With it, boot
    /// needs `MAIDAN_TURNSTILE_SITE_KEY` (the widget cannot render without
    /// one) and a hostname to hold tokens to: `MAIDAN_TURNSTILE_HOSTNAME`,
    /// or else the host of `MAIDAN_PUBLIC_ORIGIN`.
    pub fn from_env(public_origin: Option<&str>) -> Result<Option<Self>, ConfigError> {
        let Some(secret) = env_value("MAIDAN_TURNSTILE_SECRET")? else {
            if env_value("MAIDAN_TURNSTILE_SITE_KEY")?.is_some() {
                return Err(ConfigError::Invalid(
                    "MAIDAN_TURNSTILE_SITE_KEY",
                    "needs MAIDAN_TURNSTILE_SECRET; a widget whose token is never checked stops nobody".into(),
                ));
            }
            return Ok(None);
        };
        let site_key = env_value("MAIDAN_TURNSTILE_SITE_KEY")?
            .ok_or(ConfigError::Missing("MAIDAN_TURNSTILE_SITE_KEY"))?;
        let hostname = match env_value("MAIDAN_TURNSTILE_HOSTNAME")? {
            Some(h) => h.to_ascii_lowercase(),
            None => public_origin.and_then(origin_host).ok_or(ConfigError::Invalid(
                "MAIDAN_TURNSTILE_SECRET",
                "needs MAIDAN_TURNSTILE_HOSTNAME or MAIDAN_PUBLIC_ORIGIN to name the hostname tokens are solved on".into(),
            ))?,
        };
        Ok(Some(Self {
            site_key,
            secret: TurnstileSecret::new(secret),
            hostname,
            siteverify_url: SITEVERIFY_URL.to_string(),
        }))
    }
}

#[derive(Debug, Deserialize)]
struct SiteverifyAnswer {
    success: bool,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    action: Option<String>,
}

/// What the caller is told when no token came with the request.
pub const MISSING: &str = "complete the human check before continuing";
/// What the caller is told when Cloudflare did not accept the token.
pub const REJECTED: &str = "the human check was not accepted; solve it again";
/// What the caller is told when the check could not be made at all.
pub const UNAVAILABLE: &str = "the human check could not be verified right now; try again";

/// Check `token` for `action`. `Ok` only when siteverify answered success
/// for this instance's hostname and this action.
pub async fn verify(
    settings: &TurnstileSettings,
    token: Option<&str>,
    action: Action,
) -> Result<(), ApiError> {
    let token = token.map(str::trim).unwrap_or_default();
    if token.is_empty() {
        return Err(ApiError::Forbidden(MISSING.into()));
    }
    if token.chars().count() > MAX_TOKEN_CHARS {
        return Err(ApiError::Forbidden(REJECTED.into()));
    }
    let unavailable = |why: &str| {
        // The reason never carries the secret: it is a transport or parse
        // error about a request whose only secret is in its body.
        tracing::warn!(reason = %why, action = action.as_str(), "turnstile siteverify failed");
        ApiError::BadGateway(UNAVAILABLE.into())
    };
    let (client, url) = crate::egress_http::client_for(&settings.siteverify_url)
        .await
        .map_err(|e| unavailable(&e))?;
    let response = client
        .post(url)
        .form(&[("secret", settings.secret.0.as_str()), ("response", token)])
        .send()
        .await
        .map_err(|e| unavailable(&e.without_url().to_string()))?;
    if !response.status().is_success() {
        return Err(unavailable(&format!("status {}", response.status())));
    }
    let answer: SiteverifyAnswer = response
        .json()
        .await
        .map_err(|e| unavailable(&e.without_url().to_string()))?;
    let hostname_ok = answer
        .hostname
        .as_deref()
        .is_some_and(|h| h.eq_ignore_ascii_case(&settings.hostname));
    let action_ok = answer.action.as_deref() == Some(action.as_str());
    if answer.success && hostname_ok && action_ok {
        Ok(())
    } else {
        Err(ApiError::Forbidden(REJECTED.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_never_shows_in_debug_output() {
        let settings = TurnstileSettings {
            site_key: "site".into(),
            secret: TurnstileSecret::new("very-secret-value"),
            hostname: "maidan.example".into(),
            siteverify_url: SITEVERIFY_URL.into(),
        };
        assert!(!format!("{settings:?}").contains("very-secret-value"));
    }

    #[test]
    fn the_hostname_defaults_to_the_public_origins_host() {
        assert_eq!(
            origin_host("https://Maidan.Example:8443").as_deref(),
            Some("maidan.example")
        );
        assert_eq!(origin_host("not a url"), None);
    }
}
