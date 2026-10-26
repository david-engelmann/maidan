//! OIDC configuration from environment.

use std::sync::Arc;

use openidconnect::{
    core::CoreClient, ClientId, ClientSecret, EndSessionUrl, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, ProviderMetadataWithLogout, RedirectUrl,
};

/// [`CoreClient`] after provider discovery and redirect URI configuration.
pub type ConfiguredOidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;
use thiserror::Error;

use crate::config::ConfigError;
use crate::session::session_secret_from_env;

#[derive(Debug, Error)]
pub enum OidcInitError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("openid discovery failed: {0}")]
    Discovery(String),

    #[error("invalid OIDC setting: {0}")]
    Invalid(String),
}

fn env_flag(name: &'static str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

#[derive(Clone)]
pub struct OidcSettings {
    pub enabled: bool,
    pub mock: bool,
    pub issuer: String,
    pub redirect_uri: String,
    pub auto_provision: bool,
    pub link_email: bool,
    pub session_ttl_secs: u64,
    pub pending_ttl_secs: u64,
    pub cookie_secure: bool,
    pub post_logout_redirect_uri: Option<String>,
    /// When true, `POST /auth/session/mint` may create the first `token:admin`.
    pub first_admin_mint: bool,
    /// When true, redirect to `/ui/?auto_mint=1` after login if no `token:admin` yet.
    pub auto_mint: bool,
    /// When true, `GET /auth/oidc/signup` starts an invited sign-up. Off by
    /// default; boot refuses it beside OIDC auto-provisioning.
    pub signup: bool,
    /// Email domains (lower-cased) whose verified addresses may sign up
    /// without an invite. Empty unless `MAIDAN_SIGNUP_EMAIL_DOMAINS` is set.
    pub signup_email_domains: Vec<String>,
    /// When true (`MAIDAN_SIGNUP_OPEN`), any verified email may sign up
    /// without an invite, still behind the per-address and per-domain
    /// sign-up limits. Needs `MAIDAN_SIGNUP`; boot refuses it beside a
    /// domain list, which would otherwise be silently ignored.
    pub signup_open: bool,
    /// When true, an `email` claim the provider sends without
    /// `email_verified` counts as verified. For a provider whose every login
    /// proves the address, such as Cloudflare Access with its emailed
    /// one-time code; an explicit `email_verified: false` still counts as
    /// unverified.
    pub emails_verified: bool,
    /// Where the instance's privacy policy and terms live
    /// (`MAIDAN_SIGNUP_PRIVACY_URL`, `MAIDAN_SIGNUP_TERMS_URL`): a path on
    /// this origin or an `https://` URL. The waitlist page links each one
    /// that is set, since it asks visitors for an email address.
    pub privacy_url: Option<String>,
    pub terms_url: Option<String>,
}

impl OidcSettings {
    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        if !env_flag("MAIDAN_OIDC_ENABLED") {
            return Ok(None);
        }

        if std::env::var("MAIDAN_ENV").as_deref() == Ok("production")
            && env_flag("MAIDAN_OIDC_MOCK")
        {
            return Err(ConfigError::Invalid(
                "MAIDAN_OIDC_MOCK",
                "cannot be set when MAIDAN_ENV=production".into(),
            ));
        }

        if std::env::var("MAIDAN_SESSION_SECRET").is_err() {
            return Err(ConfigError::Missing("MAIDAN_SESSION_SECRET"));
        }

        let mock = env_flag("MAIDAN_OIDC_MOCK");
        let issuer = std::env::var("MAIDAN_OIDC_ISSUER").unwrap_or_else(|_| {
            if mock {
                "https://mock.idp.local".to_string()
            } else {
                String::new()
            }
        });
        let redirect_uri = std::env::var("MAIDAN_OIDC_REDIRECT_URI")
            .map_err(|_| ConfigError::Missing("MAIDAN_OIDC_REDIRECT_URI"))?;

        if !mock && issuer.is_empty() {
            return Err(ConfigError::Missing("MAIDAN_OIDC_ISSUER"));
        }

        let session_ttl_secs = std::env::var("MAIDAN_SESSION_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(28_800);
        let pending_ttl_secs = std::env::var("MAIDAN_OIDC_PENDING_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        let cookie_secure = env_flag("MAIDAN_COOKIE_SECURE")
            || std::env::var("MAIDAN_ENV").as_deref() == Ok("production");

        let post_logout_redirect_uri = std::env::var("MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI").ok();
        let first_admin_mint = !matches!(
            std::env::var("MAIDAN_OIDC_FIRST_ADMIN").as_deref(),
            Ok("0") | Ok("false") | Ok("FALSE")
        );
        let auto_mint = env_flag("MAIDAN_OIDC_AUTO_MINT") && first_admin_mint;

        // Sign-up and auto-provisioning are opposite answers to "who may
        // become a member": an invite list, or anyone the provider
        // authenticates. Running both would let the open door bypass the
        // list, so boot refuses the pair. The mock's synthetic
        // auto-provision is not the operator's flag and does not count.
        let signup = env_flag("MAIDAN_SIGNUP");
        if signup && env_flag("MAIDAN_OIDC_AUTO_PROVISION") {
            return Err(ConfigError::Invalid(
                "MAIDAN_SIGNUP",
                "cannot be set together with MAIDAN_OIDC_AUTO_PROVISION".into(),
            ));
        }
        let signup_email_domains: Vec<String> = std::env::var("MAIDAN_SIGNUP_EMAIL_DOMAINS")
            .map(|raw| {
                raw.split(',')
                    .map(|d| d.trim().to_lowercase())
                    .filter(|d| !d.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let signup_open = env_flag("MAIDAN_SIGNUP_OPEN");
        check_open_signup(signup, signup_open, !signup_email_domains.is_empty())?;

        // An operator's word stands in for the provider's: say so at boot, so
        // a login method added at the provider later is checked against it.
        let emails_verified = env_flag("MAIDAN_OIDC_EMAILS_VERIFIED");
        if emails_verified {
            tracing::warn!(
                issuer = %issuer,
                "MAIDAN_OIDC_EMAILS_VERIFIED: a missing email_verified claim counts as verified; every login method at this provider must prove the address"
            );
        }

        let privacy_url = policy_url("MAIDAN_SIGNUP_PRIVACY_URL")?;
        let terms_url = policy_url("MAIDAN_SIGNUP_TERMS_URL")?;

        Ok(Some(Self {
            enabled: true,
            mock,
            issuer,
            redirect_uri,
            auto_provision: env_flag("MAIDAN_OIDC_AUTO_PROVISION") || mock,
            link_email: env_flag("MAIDAN_OIDC_LINK_EMAIL"),
            session_ttl_secs,
            pending_ttl_secs,
            cookie_secure,
            post_logout_redirect_uri,
            first_admin_mint,
            auto_mint,
            signup,
            signup_email_domains,
            signup_open,
            emails_verified,
            privacy_url,
            terms_url,
        }))
    }
}

fn build_oidc_http_client() -> Result<reqwest::Client, OidcInitError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| OidcInitError::Invalid(e.to_string()))
}

pub struct OidcRuntime {
    pub settings: OidcSettings,
    pub session_secret: Arc<[u8]>,
    pub client: Option<ConfiguredOidcClient>,
    pub http_client: Option<Arc<reqwest::Client>>,
    pub end_session_url: Option<EndSessionUrl>,
    pub logout_client_id: Option<ClientId>,
}

impl OidcRuntime {
    pub async fn init(settings: OidcSettings) -> Result<Self, OidcInitError> {
        let session_secret = session_secret_from_env()?;
        if settings.mock {
            return Ok(Self {
                settings,
                session_secret,
                client: None,
                http_client: None,
                end_session_url: None,
                logout_client_id: None,
            });
        }

        let http_client = Arc::new(build_oidc_http_client()?);
        let issuer = IssuerUrl::new(settings.issuer.clone())
            .map_err(|e| OidcInitError::Invalid(e.to_string()))?;
        let metadata = ProviderMetadataWithLogout::discover_async(issuer, http_client.as_ref())
            .await
            .map_err(|e| OidcInitError::Discovery(e.to_string()))?;
        let end_session_url = metadata.additional_metadata().end_session_endpoint.clone();

        let client_id = ClientId::new(
            std::env::var("MAIDAN_OIDC_CLIENT_ID")
                .map_err(|_| ConfigError::Missing("MAIDAN_OIDC_CLIENT_ID"))?,
        );
        let logout_client_id = client_id.clone();
        let client_secret = std::env::var("MAIDAN_OIDC_CLIENT_SECRET")
            .ok()
            .map(ClientSecret::new);
        let redirect = RedirectUrl::new(settings.redirect_uri.clone())
            .map_err(|e| OidcInitError::Invalid(e.to_string()))?;

        let client: ConfiguredOidcClient =
            CoreClient::from_provider_metadata(metadata, client_id, client_secret)
                .set_redirect_uri(redirect);

        Ok(Self {
            settings,
            session_secret,
            client: Some(client),
            http_client: Some(http_client),
            end_session_url,
            logout_client_id: Some(logout_client_id),
        })
    }
}

/// A policy link from the environment, or `None` where it is unset or
/// blank. The waitlist page puts it in an `href`, so it is a path on this
/// origin or an `https://` URL and nothing that could leave the attribute or
/// run as script; a typo is refused at boot rather than shipped as a dead link.
fn policy_url(name: &'static str) -> Result<Option<String>, ConfigError> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(None);
    };
    check_policy_url(name, &raw)
}

fn check_policy_url(name: &'static str, raw: &str) -> Result<Option<String>, ConfigError> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let on_origin = value.starts_with('/') && !value.starts_with("//");
    // The host a visitor reads must be the one the link opens:
    // `https://maidan.world@evil.example/terms` shows one and opens the
    // other, so an authority with userinfo is refused.
    let https = value.strip_prefix("https://").is_some_and(|rest| {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        !authority.is_empty() && !authority.contains('@')
    });
    if !on_origin && !https {
        return Err(ConfigError::Invalid(
            name,
            "must be a path on this origin (`/privacy`) or an https:// URL".into(),
        ));
    }
    if value.chars().any(|c| {
        c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>' | '\\' | '`')
    }) {
        return Err(ConfigError::Invalid(
            name,
            "has a space, quote, angle bracket, backslash or control character".into(),
        ));
    }
    Ok(Some(value.to_string()))
}

/// Open sign-up widens the invited one, so it means nothing without
/// `MAIDAN_SIGNUP`, and a domain list beside it would be ignored: refuse
/// both at boot rather than run a door the operator did not describe.
fn check_open_signup(signup: bool, open: bool, has_domains: bool) -> Result<(), ConfigError> {
    if !open {
        return Ok(());
    }
    if !signup {
        return Err(ConfigError::Invalid(
            "MAIDAN_SIGNUP_OPEN",
            "needs MAIDAN_SIGNUP=1".into(),
        ));
    }
    if has_domains {
        return Err(ConfigError::Invalid(
            "MAIDAN_SIGNUP_OPEN",
            "cannot be set together with MAIDAN_SIGNUP_EMAIL_DOMAINS".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod policy_url_tests {
    use super::*;

    #[test]
    fn a_policy_link_is_a_path_on_this_origin_or_an_https_url() {
        for ok in [
            "/privacy",
            "/legal/terms.html",
            "https://example.com/privacy?v=2&x=1",
        ] {
            assert_eq!(
                check_policy_url("MAIDAN_SIGNUP_PRIVACY_URL", ok)
                    .ok()
                    .flatten()
                    .as_deref(),
                Some(ok)
            );
        }
        assert_eq!(
            check_policy_url("MAIDAN_SIGNUP_PRIVACY_URL", "  ")
                .ok()
                .flatten(),
            None
        );
    }

    #[test]
    fn a_policy_link_that_could_leave_the_attribute_or_the_origin_is_refused() {
        for bad in [
            "privacy",
            "//evil.example/privacy",
            "http://example.com/privacy",
            "https:///privacy",
            "javascript:alert(1)",
            "/privacy\" onmouseover=\"x",
            "/privacy><script>",
            "/pri vacy",
            "/privacy'",
            "/a\\b",
            "https://user:pass@example.com/privacy",
            "https://maidan.world@evil.example/terms",
            "https://?x",
        ] {
            assert!(
                matches!(
                    check_policy_url("MAIDAN_SIGNUP_TERMS_URL", bad),
                    Err(ConfigError::Invalid("MAIDAN_SIGNUP_TERMS_URL", _))
                ),
                "{bad}"
            );
        }
    }
}

#[cfg(test)]
mod open_signup_tests {
    use super::*;

    fn refused_var(r: Result<(), ConfigError>) -> Option<&'static str> {
        match r {
            Err(ConfigError::Invalid(var, _)) => Some(var),
            _ => None,
        }
    }

    #[test]
    fn open_signup_is_refused_without_signup_itself() {
        assert_eq!(
            refused_var(check_open_signup(false, true, false)),
            Some("MAIDAN_SIGNUP_OPEN")
        );
    }

    #[test]
    fn open_signup_is_refused_beside_a_domain_list() {
        assert_eq!(
            refused_var(check_open_signup(true, true, true)),
            Some("MAIDAN_SIGNUP_OPEN")
        );
    }

    #[test]
    fn open_signup_with_signup_and_no_domain_list_boots() {
        assert!(check_open_signup(true, true, false).is_ok());
        assert!(check_open_signup(false, false, true).is_ok());
    }
}
