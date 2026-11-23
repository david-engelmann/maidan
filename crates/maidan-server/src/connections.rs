//! A workspace's own GitHub, Slack and Linear connections, and the one place
//! that decides whose credential acts for a workspace.
//!
//! A workspace is served on a surface when it stores its own connection, or
//! when the operator opened it and the server holds an instance credential
//! (`maidan_store::instance_credentials` holds the store's half of the rule).
//! When the workspace stores its own, that credential is used, always. The
//! instance's is never used for a workspace the operator did not open.
//!
//! A connection is one reserved workspace secret per surface
//! ([`ConnectionSurface::secret_name`]), encrypted like every workspace
//! secret and out of reach of the generic secret routes. Every place that
//! reads or writes an outside service for a workspace asks [`AppState`] for
//! its sender here, so no path can reach for the instance's by habit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use maidan_types::{ConnectionSurface, WorkspaceId, WorkspaceOrigin};
use serde::Deserialize;

use crate::github::GithubSender;
use crate::linear::LinearSender;
use crate::slack::SlackSender;
use crate::state::AppState;

/// Whose credential a sender holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The workspace stored it.
    Workspace,
    /// The server's own, for a workspace its operator opened.
    Instance,
}

/// A sender and whose credential it holds.
pub struct Resolved<T: ?Sized> {
    pub sender: Arc<T>,
    pub source: Source,
}

impl<T: ?Sized> Clone for Resolved<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            source: self.source,
        }
    }
}

/// Why a workspace's sender could not be settled. Not "no credential serves
/// it": that is `Ok(None)`.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error(transparent)]
    Store(#[from] maidan_store::StoreError),
    /// The workspace stored a connection that cannot be opened, so none is
    /// used, and the instance's is not lent in its place.
    #[error("this workspace's {surface} connection cannot be opened: {reason}")]
    Unreadable {
        surface: ConnectionSurface,
        reason: String,
    },
}

/// A workspace's stored connection. The token and the inbound secret are
/// credentials: `{:?}` never prints them.
#[derive(Clone, Deserialize)]
pub struct StoredConnection {
    pub token: String,
    #[serde(default)]
    pub account: String,
    /// GitHub: verifies `X-Hub-Signature-256` on the workspace's webhook.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Slack: verifies the request signature on the workspace's events URL.
    #[serde(default)]
    pub signing_secret: Option<String>,
    pub connected_at: DateTime<Utc>,
}

// Written out rather than derived, as `SlackConfig` is.
impl std::fmt::Debug for StoredConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredConnection")
            .field("token", &"***")
            .field("account", &self.account)
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "***"),
            )
            .field(
                "signing_secret",
                &self.signing_secret.as_ref().map(|_| "***"),
            )
            .field("connected_at", &self.connected_at)
            .finish()
    }
}

impl StoredConnection {
    /// The inbound secret this surface verifies deliveries with.
    pub fn inbound_secret(&self, surface: ConnectionSurface) -> Option<&str> {
        match surface {
            ConnectionSurface::Github => self.webhook_secret.as_deref(),
            ConnectionSurface::Slack => self.signing_secret.as_deref(),
            ConnectionSurface::Linear => None,
        }
    }

    /// The JSON the secret holds: the keys the surface has, and no others.
    pub fn to_plaintext(&self, surface: ConnectionSurface) -> String {
        let mut value = serde_json::json!({
            "token": self.token,
            "account": self.account,
            "connected_at": self.connected_at.to_rfc3339(),
        });
        match surface {
            ConnectionSurface::Github => {
                value["webhook_secret"] = self.webhook_secret.clone().into()
            }
            ConnectionSurface::Slack => {
                value["signing_secret"] = self.signing_secret.clone().into()
            }
            ConnectionSurface::Linear => {}
        }
        value.to_string()
    }
}

/// Why a token was not accepted for storing.
#[derive(Debug)]
pub enum VerifyError {
    /// The provider refused the token. The text is the provider's own reason.
    Refused(String),
    /// The provider could not be asked, or answered something unusable.
    Unavailable(String),
}

/// What builds a workspace's clients from its token and asks a provider who a
/// token belongs to. The live one talks to the providers; tests replace it
/// with one that returns mocks ([`AppState::attach_connection_factory`]).
#[async_trait::async_trait]
pub trait ConnectionFactory: Send + Sync {
    /// A GitHub client for a workspace's token. It writes wherever the token
    /// reaches: the workspace's allowlist and the token's own permissions
    /// bound it, where `MAIDAN_GITHUB_WRITE_REPOS` bounds the instance's.
    fn github(&self, token: &str) -> Arc<dyn GithubSender>;
    fn slack(&self, token: &str) -> Arc<dyn SlackSender>;
    fn linear(&self, token: &str) -> Arc<dyn LinearSender>;
    /// The account `token` belongs to, from one identity call to the provider.
    async fn verify(&self, surface: ConnectionSurface, token: &str) -> Result<String, VerifyError>;
}

/// The factory the server runs with.
pub struct LiveConnections;

#[async_trait::async_trait]
impl ConnectionFactory for LiveConnections {
    fn github(&self, token: &str) -> Arc<dyn GithubSender> {
        Arc::new(crate::github::GithubApiClient::new(token.to_string()).with_any_write_repo())
    }

    fn slack(&self, token: &str) -> Arc<dyn SlackSender> {
        Arc::new(crate::slack::SlackWebClient::new(token.to_string()))
    }

    fn linear(&self, token: &str) -> Arc<dyn LinearSender> {
        Arc::new(crate::linear::LinearClient::new(token.to_string()))
    }

    async fn verify(&self, surface: ConnectionSurface, token: &str) -> Result<String, VerifyError> {
        // A refusal is the provider saying the token is no good, which the
        // caller can fix. Anything else is a fault in reaching or reading it.
        let classify = |refused: bool, text: String| {
            if refused {
                VerifyError::Refused(text)
            } else {
                VerifyError::Unavailable(text)
            }
        };
        match surface {
            ConnectionSurface::Github => crate::github::GithubApiClient::new(token.to_string())
                .identity()
                .await
                .map_err(|e| classify(e.is_misconfiguration(), e.to_string())),
            ConnectionSurface::Slack => crate::slack::SlackWebClient::new(token.to_string())
                .identity()
                .await
                .map_err(|e| classify(e.is_misconfiguration(), e.to_string())),
            ConnectionSurface::Linear => crate::linear::LinearClient::new(token.to_string())
                .identity()
                .await
                .map_err(|e| classify(e.is_misconfiguration(), e.to_string())),
        }
    }
}

/// A built client, kept with the ciphertext it was built from.
#[derive(Clone)]
enum Built {
    Github(Arc<dyn GithubSender>),
    Slack(Arc<dyn SlackSender>),
    Linear(Arc<dyn LinearSender>),
}

/// The clients built from workspaces' tokens. Building one loads a TLS
/// client, so it is kept until the stored connection changes. Each entry
/// names the ciphertext it was built from, so a replaced or removed
/// connection is never served from here.
#[derive(Default)]
pub struct ConnectionCache {
    entries: Mutex<HashMap<(WorkspaceId, ConnectionSurface), (String, Built)>>,
}

/// More workspaces than this have their clients dropped and rebuilt on use,
/// so the cache cannot grow with the number of tenants.
const CACHE_LIMIT: usize = 1024;

impl ConnectionCache {
    fn get(&self, key: (WorkspaceId, ConnectionSurface), ciphertext: &str) -> Option<Built> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(&key)
            .filter(|(held, _)| held == ciphertext)
            .map(|(_, built)| built.clone())
    }

    fn put(&self, key: (WorkspaceId, ConnectionSurface), ciphertext: String, built: Built) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= CACHE_LIMIT {
            entries.clear();
        }
        entries.insert(key, (ciphertext, built));
    }
}

impl AppState {
    /// The workspace's stored connection for `surface`, with the ciphertext it
    /// was read as. `None` when it stored none.
    pub async fn stored_connection(
        &self,
        workspace_id: WorkspaceId,
        surface: ConnectionSurface,
    ) -> Result<Option<(String, StoredConnection)>, ConnectionError> {
        let Some(ciphertext) = self
            .store
            .get_secret_ciphertext(workspace_id, surface.secret_name())
            .await?
        else {
            return Ok(None);
        };
        let unreadable = |reason: String| ConnectionError::Unreadable { surface, reason };
        let key = self.federation.encryption_key.as_deref().ok_or_else(|| {
            unreadable("this instance has no key to open workspace secrets with".into())
        })?;
        let plaintext = maidan_auth::decrypt_peer_secret_rotating(&ciphertext, key)
            .map_err(|e| unreadable(e.to_string()))?;
        let connection: StoredConnection =
            serde_json::from_str(&plaintext).map_err(|e| unreadable(e.to_string()))?;
        Ok(Some((ciphertext, connection)))
    }

    /// Whether the instance's credential may serve this workspace: the
    /// operator opened it.
    async fn instance_serves(&self, workspace_id: WorkspaceId) -> Result<bool, ConnectionError> {
        Ok(self.store.workspace_origin(workspace_id).await? == WorkspaceOrigin::Operator)
    }

    /// The GitHub sender that acts for this workspace, and whose credential it
    /// holds. `None` when no credential serves the workspace on GitHub.
    pub async fn github_for(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<Resolved<dyn GithubSender>>, ConnectionError> {
        let surface = ConnectionSurface::Github;
        if let Some((ciphertext, connection)) =
            self.stored_connection(workspace_id, surface).await?
        {
            let key = (workspace_id, surface);
            let sender = match self.connection_cache.get(key, &ciphertext) {
                Some(Built::Github(sender)) => sender,
                _ => {
                    let sender = self.connection_factory.github(&connection.token);
                    self.connection_cache
                        .put(key, ciphertext, Built::Github(sender.clone()));
                    sender
                }
            };
            return Ok(Some(Resolved {
                sender,
                source: Source::Workspace,
            }));
        }
        if !self.instance_serves(workspace_id).await? {
            return Ok(None);
        }
        Ok(self.github_sender.clone().map(|sender| Resolved {
            sender,
            source: Source::Instance,
        }))
    }

    /// [`Self::github_for`] for Slack.
    pub async fn slack_for(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<Resolved<dyn SlackSender>>, ConnectionError> {
        let surface = ConnectionSurface::Slack;
        if let Some((ciphertext, connection)) =
            self.stored_connection(workspace_id, surface).await?
        {
            let key = (workspace_id, surface);
            let sender = match self.connection_cache.get(key, &ciphertext) {
                Some(Built::Slack(sender)) => sender,
                _ => {
                    let sender = self.connection_factory.slack(&connection.token);
                    self.connection_cache
                        .put(key, ciphertext, Built::Slack(sender.clone()));
                    sender
                }
            };
            return Ok(Some(Resolved {
                sender,
                source: Source::Workspace,
            }));
        }
        if !self.instance_serves(workspace_id).await? {
            return Ok(None);
        }
        Ok(self.slack_sender.clone().map(|sender| Resolved {
            sender,
            source: Source::Instance,
        }))
    }

    /// [`Self::github_for`] for Linear.
    pub async fn linear_for(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<Resolved<dyn LinearSender>>, ConnectionError> {
        let surface = ConnectionSurface::Linear;
        if let Some((ciphertext, connection)) =
            self.stored_connection(workspace_id, surface).await?
        {
            let key = (workspace_id, surface);
            let sender = match self.connection_cache.get(key, &ciphertext) {
                Some(Built::Linear(sender)) => sender,
                _ => {
                    let sender = self.connection_factory.linear(&connection.token);
                    self.connection_cache
                        .put(key, ciphertext, Built::Linear(sender.clone()));
                    sender
                }
            };
            return Ok(Some(Resolved {
                sender,
                source: Source::Workspace,
            }));
        }
        if !self.instance_serves(workspace_id).await? {
            return Ok(None);
        }
        Ok(self.linear_sender.clone().map(|sender| Resolved {
            sender,
            source: Source::Instance,
        }))
    }

    /// Who serves this workspace on `surface`, for a listing: its own
    /// connection, the instance's credential, or nobody. Reads no secret
    /// value beyond what a row shows.
    pub async fn connection_source(
        &self,
        workspace_id: WorkspaceId,
        surface: ConnectionSurface,
    ) -> Result<Option<Source>, ConnectionError> {
        if self
            .store
            .get_secret_ciphertext(workspace_id, surface.secret_name())
            .await?
            .is_some()
        {
            return Ok(Some(Source::Workspace));
        }
        let held = match surface {
            ConnectionSurface::Github => self.github_sender.is_some(),
            ConnectionSurface::Slack => self.slack_sender.is_some(),
            ConnectionSurface::Linear => self.linear_sender.is_some(),
        };
        Ok((held && self.instance_serves(workspace_id).await?).then_some(Source::Instance))
    }

    /// Whether any credential could serve some workspace on any surface: the
    /// server's own, or a key to open workspaces' own. A worker with neither
    /// has nothing it could send.
    pub fn may_send_anywhere(&self) -> bool {
        self.slack_sender.is_some()
            || self.github_sender.is_some()
            || self.linear_sender.is_some()
            || self.federation.encryption_key.is_some()
    }

    /// Replace the factory that builds workspaces' clients and checks their
    /// tokens. Tests inject one that returns mocks.
    pub fn attach_connection_factory(&mut self, factory: Arc<dyn ConnectionFactory>) {
        self.connection_factory = factory;
        self.connection_cache = Arc::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_connection_never_prints_its_token_or_its_inbound_secret() {
        let connection = StoredConnection {
            token: "ghp_not_a_real_token".into(),
            account: "octocat".into(),
            webhook_secret: Some("not-a-real-webhook-secret".into()),
            signing_secret: Some("not-a-real-signing-secret".into()),
            connected_at: Utc::now(),
        };
        let printed = format!("{connection:?}");
        assert!(!printed.contains("ghp_not_a_real_token"));
        assert!(!printed.contains("not-a-real-webhook-secret"));
        assert!(!printed.contains("not-a-real-signing-secret"));
        assert!(printed.contains("octocat"));
    }

    #[test]
    fn a_connection_round_trips_with_only_its_surfaces_keys() {
        let connection = StoredConnection {
            token: "t".into(),
            account: "a".into(),
            webhook_secret: Some("w".into()),
            signing_secret: Some("s".into()),
            connected_at: Utc::now(),
        };
        let github: serde_json::Value =
            serde_json::from_str(&connection.to_plaintext(ConnectionSurface::Github)).unwrap();
        assert_eq!(github["webhook_secret"], "w");
        assert!(github.get("signing_secret").is_none());
        let linear: serde_json::Value =
            serde_json::from_str(&connection.to_plaintext(ConnectionSurface::Linear)).unwrap();
        assert!(linear.get("webhook_secret").is_none());
        assert!(linear.get("signing_secret").is_none());
        let back: StoredConnection =
            serde_json::from_str(&connection.to_plaintext(ConnectionSurface::Slack)).unwrap();
        assert_eq!(back.signing_secret.as_deref(), Some("s"));
        assert_eq!(back.token, "t");
    }
}
