//! Outbound GitHub calls authenticated as a GitHub App (Open Work Next 22).
//!
//! `MAIDAN_GITHUB_TOKEN` is one long-lived credential, usually a person's
//! token, so everything Maidan writes to GitHub is authored as that person.
//! With `MAIDAN_GITHUB_APP_ID`, `MAIDAN_GITHUB_APP_INSTALLATION_ID` and
//! `MAIDAN_GITHUB_APP_PRIVATE_KEY` set, the client authenticates as the app
//! instead:
//!
//! 1. It signs a JWT with the app's private key (RS256, issued 60 s in the
//!    past for clock drift, nine minutes long, under GitHub's ten-minute cap).
//! 2. It exchanges the JWT at `POST /app/installations/{id}/access_tokens` for
//!    an installation token. That token reaches only the repositories the
//!    installation was granted, with the permissions the app was granted, and
//!    lasts an hour.
//! 3. It keeps that token until five minutes before its `expires_at`, then
//!    exchanges again. One exchange runs at a time, so a burst of deliveries
//!    after expiry makes one call to GitHub, not one each.
//!
//! The installation token is used exactly where the configured token was:
//! `MAIDAN_GITHUB_WRITE_REPOS` still bounds every write before a request is
//! built. The signing uses `ring`, not the `rsa` crate, whose private-key
//! operations are what `RUSTSEC-2023-0071` is about.

use std::sync::{Mutex, MutexGuard, PoisonError};

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use ring::rand::SystemRandom;
use ring::rsa::KeyPair as RsaKeyPair;
use ring::signature::RSA_PKCS1_SHA256;

use crate::github::{is_rate_limited, GithubError};

/// The app's id (a number) or client id (`Iv23…`). GitHub accepts either as
/// the JWT's `iss`.
pub const APP_ID_ENV: &str = "MAIDAN_GITHUB_APP_ID";
/// The installation whose token the app exchanges for.
pub const INSTALLATION_ID_ENV: &str = "MAIDAN_GITHUB_APP_INSTALLATION_ID";
/// The app's private key, PEM (PKCS#1 as GitHub downloads it, or PKCS#8).
pub const PRIVATE_KEY_ENV: &str = "MAIDAN_GITHUB_APP_PRIVATE_KEY";

/// A cached installation token is replaced this long before it expires, so a
/// request built from it does not reach GitHub after the expiry.
pub const REFRESH_MARGIN: Duration = Duration::minutes(5);
/// The JWT's `iat` is this far in the past, as GitHub recommends for drift.
const JWT_BACKDATE_SECS: i64 = 60;
/// The JWT's `exp` is this far after now. GitHub refuses more than ten minutes.
const JWT_LIFETIME_SECS: i64 = 9 * 60;

/// The app's credentials and the installation token it last exchanged for.
pub struct GithubAppAuth {
    /// Emitted as a JSON number when it is all digits (an app id), as a
    /// string otherwise (a client id).
    app_id: String,
    installation_id: u64,
    key: RsaKeyPair,
    rng: SystemRandom,
    current: Mutex<Option<InstallationToken>>,
    /// Held across an exchange, so concurrent callers wait for one exchange
    /// instead of each starting their own.
    exchange: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
struct InstallationToken {
    token: String,
    /// When this token stops being handed out: `expires_at` less
    /// [`REFRESH_MARGIN`].
    refresh_after: DateTime<Utc>,
}

// The key and the token are credentials; `{:?}` must never print them.
impl std::fmt::Debug for GithubAppAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubAppAuth")
            .field("app_id", &self.app_id)
            .field("installation_id", &self.installation_id)
            .field("key", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl GithubAppAuth {
    /// The app named by the environment, `None` when none of the three
    /// variables is set. Setting some but not all of them, or a value that
    /// does not parse, is an error naming the variable and never its value:
    /// boot refuses it rather than falling back to `MAIDAN_GITHUB_TOKEN`
    /// without a word.
    pub fn from_env() -> Result<Option<GithubAppAuth>, String> {
        let read = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let app_id = read(APP_ID_ENV);
        let installation_id = read(INSTALLATION_ID_ENV);
        let key = read(PRIVATE_KEY_ENV);
        match (app_id, installation_id, key) {
            (None, None, None) => Ok(None),
            (Some(app_id), Some(installation_id), Some(key)) => {
                Self::new(&app_id, &installation_id, &key).map(Some)
            }
            (app_id, installation_id, key) => {
                let missing: Vec<&str> = [
                    (APP_ID_ENV, app_id.is_none()),
                    (INSTALLATION_ID_ENV, installation_id.is_none()),
                    (PRIVATE_KEY_ENV, key.is_none()),
                ]
                .into_iter()
                .filter_map(|(name, missing)| missing.then_some(name))
                .collect();
                Err(format!(
                    "a GitHub App needs {APP_ID_ENV}, {INSTALLATION_ID_ENV} and \
                     {PRIVATE_KEY_ENV} together; {} unset",
                    missing.join(" and ")
                ))
            }
        }
    }

    /// Parse the three values. Errors name the variable, never the value.
    pub fn new(
        app_id: &str,
        installation_id: &str,
        private_key_pem: &str,
    ) -> Result<GithubAppAuth, String> {
        let app_id = app_id.trim();
        let valid_id = !app_id.is_empty()
            && app_id.len() <= 64
            && app_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
        if !valid_id {
            return Err(format!("{APP_ID_ENV} is not a GitHub App id or client id"));
        }
        let installation_id = installation_id
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| format!("{INSTALLATION_ID_ENV} is not an installation id"))?;
        let key = parse_private_key(private_key_pem)?;
        Ok(GithubAppAuth {
            app_id: app_id.to_string(),
            installation_id,
            key,
            rng: SystemRandom::new(),
            current: Mutex::new(None),
            exchange: tokio::sync::Mutex::new(()),
        })
    }

    pub fn installation_id(&self) -> u64 {
        self.installation_id
    }

    /// An installation token to send as the bearer: the cached one while it
    /// is fresh, otherwise a new one from GitHub.
    pub(crate) async fn installation_token(
        &self,
        http: &reqwest::Client,
        base_url: &str,
    ) -> Result<String, GithubError> {
        if let Some(token) = self.fresh(Utc::now()) {
            return Ok(token);
        }
        let _one_at_a_time = self.exchange.lock().await;
        // Another caller may have exchanged while this one waited.
        if let Some(token) = self.fresh(Utc::now()) {
            return Ok(token);
        }
        let issued = self.exchange_jwt(http, base_url).await?;
        let token = issued.token.clone();
        *self.cache() = Some(issued);
        Ok(token)
    }

    /// `text` with the cached installation token cut out.
    pub(crate) fn redact(&self, text: &str) -> String {
        let token = self.cache().as_ref().map(|t| t.token.clone());
        match token {
            Some(token) => crate::github::redact(text, &token),
            None => text.to_string(),
        }
    }

    /// Cache `token` as if GitHub had just issued it for an hour.
    #[cfg(test)]
    pub(crate) fn seed_token_for_test(&self, token: &str) {
        *self.cache() = Some(InstallationToken {
            token: token.to_string(),
            refresh_after: Utc::now() + Duration::minutes(55),
        });
    }

    /// The token cache. A panic while it was held leaves at worst a stale
    /// token, which `fresh` already judges by its time, so a poisoned lock
    /// is taken anyway: skipping it would exchange a JWT on every request
    /// and stop redacting the token.
    fn cache(&self) -> MutexGuard<'_, Option<InstallationToken>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fresh(&self, now: DateTime<Utc>) -> Option<String> {
        self.cache()
            .as_ref()
            .filter(|t| now < t.refresh_after)
            .map(|t| t.token.clone())
    }

    async fn exchange_jwt(
        &self,
        http: &reqwest::Client,
        base_url: &str,
    ) -> Result<InstallationToken, GithubError> {
        let jwt = self.jwt(Utc::now())?;
        let url = format!(
            "{base_url}/app/installations/{}/access_tokens",
            self.installation_id
        );
        let failed =
            |err: reqwest::Error| GithubError::Http(crate::github::redact(&err.to_string(), &jwt));
        let resp = crate::trace_context::stamp(http.post(url))
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "maidan-projector")
            .send()
            .await
            .map_err(failed)?;
        let status = resp.status();
        if !status.is_success() {
            let rate_limited = is_rate_limited(resp.headers());
            if rate_limited || status.is_server_error() || status.as_u16() == 429 {
                return Err(GithubError::Api {
                    status: status.as_u16(),
                    rate_limited,
                });
            }
            // Not an `Api` error: a 404 here means the installation is gone,
            // not that the comment or pull request a caller asked about is,
            // and the callers read a 404 as the latter.
            return Err(GithubError::Refused(format!(
                "GitHub refused the app's installation token exchange ({}): check \
                 {APP_ID_ENV}, {INSTALLATION_ID_ENV} and the app's key",
                status.as_u16()
            )));
        }
        let body: serde_json::Value = resp.json().await.map_err(failed)?;
        let token = body
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                GithubError::Http("github installation token response has no token".into())
            })?
            .to_string();
        let expires_at = body
            .get("expires_at")
            .and_then(serde_json::Value::as_str)
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.with_timezone(&Utc))
            .ok_or_else(|| {
                GithubError::Http("github installation token response has no expires_at".into())
            })?;
        Ok(InstallationToken {
            token,
            refresh_after: expires_at - REFRESH_MARGIN,
        })
    }

    /// The app's JWT at `now`: `iat` a minute back, `exp` nine minutes on,
    /// `iss` the app.
    fn jwt(&self, now: DateTime<Utc>) -> Result<String, GithubError> {
        let iss = match self.app_id.parse::<u64>() {
            Ok(id) => serde_json::json!(id),
            Err(_) => serde_json::json!(self.app_id),
        };
        let at = now.timestamp();
        let header = serde_json::json!({ "alg": "RS256", "typ": "JWT" });
        let claims = serde_json::json!({
            "iat": at - JWT_BACKDATE_SECS,
            "exp": at + JWT_LIFETIME_SECS,
            "iss": iss,
        });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let signing_input = format!(
            "{}.{}",
            b64.encode(header.to_string()),
            b64.encode(claims.to_string())
        );
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &self.rng,
                signing_input.as_bytes(),
                &mut signature,
            )
            .map_err(|_| GithubError::Http("could not sign the GitHub App JWT".into()))?;
        Ok(format!("{signing_input}.{}", b64.encode(signature)))
    }
}

/// A PEM private key: `RSA PRIVATE KEY` (PKCS#1, what GitHub downloads) or
/// `PRIVATE KEY` (PKCS#8). A key pasted into an environment variable often
/// arrives with its newlines written as `\n`, so those are accepted too.
fn parse_private_key(pem: &str) -> Result<RsaKeyPair, String> {
    let unusable = || format!("{PRIVATE_KEY_ENV} is not an RSA private key in PEM form");
    let pem = if pem.contains('\n') {
        pem.to_string()
    } else {
        pem.replace("\\n", "\n")
    };
    let (label, body) = pem_body(&pem).ok_or_else(unusable)?;
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|_| unusable())?;
    let parsed = match label.as_str() {
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(&der),
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(&der),
        _ => return Err(unusable()),
    };
    parsed.map_err(|_| {
        format!("{PRIVATE_KEY_ENV} is not a usable RSA private key (2048 bits or more)")
    })
}

/// The label and the base64 body of the first PEM block in `pem`.
fn pem_body(pem: &str) -> Option<(String, String)> {
    let mut lines = pem
        .lines()
        .map(str::trim)
        .skip_while(|l| !l.starts_with("-----BEGIN "));
    let label = lines
        .next()?
        .strip_prefix("-----BEGIN ")?
        .strip_suffix("-----")?
        .to_string();
    let end = format!("-----END {label}-----");
    let mut body = String::new();
    for line in lines {
        if line == end {
            return Some((label, body));
        }
        body.push_str(line);
    }
    None
}

/// The throwaway key the tests sign with, shared with the integration tests.
#[cfg(test)]
#[path = "../tests/github_app_key/mod.rs"]
pub(crate) mod github_app_key;

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{KeyPair as _, UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256};

    use super::github_app_key;

    fn pkcs1() -> &'static str {
        &github_app_key::test_key().pkcs1_pem
    }

    fn decode(part: &str) -> Vec<u8> {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part)
            .unwrap()
    }

    #[test]
    fn the_jwt_is_rs256_signed_by_the_app_key_and_lives_under_ten_minutes() {
        let app = GithubAppAuth::new("12345", "678", pkcs1()).unwrap();
        let now = Utc::now();
        let jwt = app.jwt(now).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "{jwt}");
        let header: serde_json::Value = serde_json::from_slice(&decode(parts[0])).unwrap();
        assert_eq!(header["alg"], "RS256");
        let claims: serde_json::Value = serde_json::from_slice(&decode(parts[1])).unwrap();
        assert_eq!(
            claims["iss"],
            serde_json::json!(12345),
            "an app id is a number"
        );
        let iat = claims["iat"].as_i64().unwrap();
        let exp = claims["exp"].as_i64().unwrap();
        assert_eq!(iat, now.timestamp() - 60);
        assert!(
            exp - iat <= 600,
            "GitHub refuses a JWT longer than ten minutes"
        );
        assert!(exp > now.timestamp());

        let public = app.key.public_key().as_ref().to_vec();
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public)
            .verify(
                format!("{}.{}", parts[0], parts[1]).as_bytes(),
                &decode(parts[2]),
            )
            .expect("the signature verifies with the app's public key");
    }

    #[test]
    fn a_client_id_is_the_issuer_as_a_string() {
        let app = GithubAppAuth::new("Iv23liExample", "678", pkcs1()).unwrap();
        let jwt = app.jwt(Utc::now()).unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&decode(jwt.split('.').nth(1).unwrap())).unwrap();
        assert_eq!(claims["iss"], "Iv23liExample");
    }

    #[test]
    fn pkcs8_and_escaped_newlines_parse_too() {
        assert!(GithubAppAuth::new("1", "2", &github_app_key::test_key().pkcs8_pem).is_ok());
        let one_line = pkcs1().trim_end().replace('\n', "\\n");
        assert!(!one_line.contains('\n'));
        assert!(GithubAppAuth::new("1", "2", &one_line).is_ok());
    }

    #[test]
    fn bad_values_are_refused_naming_the_variable_not_the_value() {
        for (app, inst, key, names) in [
            ("", "2", pkcs1(), APP_ID_ENV),
            ("12 34", "2", pkcs1(), APP_ID_ENV),
            ("1", "0", pkcs1(), INSTALLATION_ID_ENV),
            ("1", "abc", pkcs1(), INSTALLATION_ID_ENV),
            ("1", "2", "not a key", PRIVATE_KEY_ENV),
            (
                "1",
                "2",
                "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----",
                PRIVATE_KEY_ENV,
            ),
        ] {
            let err = GithubAppAuth::new(app, inst, key).unwrap_err();
            assert!(err.contains(names), "{err}");
            assert!(!err.contains("BEGIN"), "{err}");
        }
    }

    #[test]
    fn the_key_never_reaches_debug_output() {
        let app = GithubAppAuth::new("1", "2", pkcs1()).unwrap();
        let shown = format!("{app:?}");
        assert!(
            shown.contains("[redacted]") && !shown.contains("BEGIN"),
            "{shown}"
        );
    }

    #[test]
    fn a_cached_token_is_handed_out_until_the_refresh_margin() {
        let app = GithubAppAuth::new("1", "2", pkcs1()).unwrap();
        let now = Utc::now();
        *app.current.lock().unwrap() = Some(InstallationToken {
            token: "ghs_cached".into(),
            refresh_after: now + Duration::minutes(1),
        });
        assert_eq!(app.fresh(now).as_deref(), Some("ghs_cached"));
        assert_eq!(app.fresh(now + Duration::minutes(1)), None);
        assert_eq!(app.redact("x ghs_cached y"), "x [redacted] y");
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods)] // a fake GitHub, not the API
    async fn a_poisoned_cache_still_caches_and_redacts() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let exchanges = Arc::new(AtomicUsize::new(0));
        let counted = exchanges.clone();
        let fake = axum::Router::new().route(
            "/app/installations/2/access_tokens",
            axum::routing::post(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                let expires_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
                async move {
                    axum::Json(serde_json::json!({
                        "token": "ghs_afterpanic",
                        "expires_at": expires_at,
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });

        let app = Arc::new(GithubAppAuth::new("1", "2", pkcs1()).unwrap());
        let poisoner = app.clone();
        let _ = std::thread::spawn(move || {
            let _held = poisoner.current.lock();
            panic!("poison the token cache");
        })
        .join();
        assert!(app.current.is_poisoned());

        let http = reqwest::Client::new();
        for _ in 0..3 {
            let token = app.installation_token(&http, &base).await.unwrap();
            assert_eq!(token, "ghs_afterpanic");
        }
        assert_eq!(
            exchanges.load(Ordering::SeqCst),
            1,
            "one exchange, then the cache"
        );
        assert_eq!(app.redact("x ghs_afterpanic y"), "x [redacted] y");
    }
}
