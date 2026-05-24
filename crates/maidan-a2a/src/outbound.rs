//! HTTP client for pulling events from a remote Maidan peer.

use maidan_types::{Peer, PeerId, StoredEvent};
use reqwest::Client;

use crate::error::FederationError;

#[derive(Debug, Clone)]
pub struct Outbound {
    client: Client,
}

impl Default for Outbound {
    fn default() -> Self {
        Self::new()
    }
}

impl Outbound {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Poll the remote peer's event log after `after_id` (exclusive).
    pub async fn list_events(
        &self,
        peer: &Peer,
        bearer: &str,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, FederationError> {
        let base = peer.base_url.trim_end_matches('/');
        let url = format!(
            "{base}/workspaces/{}/events?after_id={after_id}&limit={limit}",
            peer.remote_workspace_id.0
        );
        let response = self
            .client
            .get(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .map_err(|e| FederationError::Transport(e.to_string()))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(FederationError::Unauthorized);
        }
        if !response.status().is_success() {
            return Err(FederationError::Transport(format!(
                "remote list_events returned {}",
                response.status()
            )));
        }

        response
            .json::<Vec<StoredEvent>>()
            .await
            .map_err(|e| FederationError::Transport(e.to_string()))
    }

    /// Push a batch to the remote peer's federation ingress.
    pub async fn push_batch(
        &self,
        peer: &Peer,
        bearer: &str,
        body: &crate::FederatedEventBatch,
    ) -> Result<(), FederationError> {
        let base = peer.base_url.trim_end_matches('/');
        let url = format!("{base}/a2a/v1/events");
        let response = self
            .client
            .post(&url)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await
            .map_err(|e| FederationError::Transport(e.to_string()))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(FederationError::Unauthorized);
        }
        if !response.status().is_success() {
            return Err(FederationError::Transport(format!(
                "remote ingest returned {}",
                response.status()
            )));
        }
        Ok(())
    }
}

impl Outbound {
    /// Build envelopes from stored rows for a given origin peer id.
    pub fn to_envelopes(
        origin_peer_id: PeerId,
        events: Vec<StoredEvent>,
    ) -> Vec<crate::FederationEnvelope> {
        events
            .into_iter()
            .map(|event| {
                let remote_event_id = event.id;
                crate::FederationEnvelope {
                    origin_peer_id,
                    remote_event_id,
                    event,
                }
            })
            .collect()
    }
}
