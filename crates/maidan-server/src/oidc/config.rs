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
