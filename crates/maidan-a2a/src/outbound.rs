//! HTTP client for pulling events from a remote Maidan peer.

use maidan_types::{Peer, PeerId, StoredEvent};
use reqwest::Client;

use crate::error::FederationError;

#[derive(Debug, Clone)]
pub struct Outbound {
    client: Client,
}

impl Outbound {
    /// Poll through `client`, which must be the egress-guarded client for the
    /// peer's URL (`egress_http::client_for`): a peer's base URL is
    /// operator-supplied, so a default client would reach internal addresses
    /// and wait forever on a peer that never answers.
    pub fn new(client: Client) -> Self {
        Self { client }
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
            return Err(transport_status(response, "list_events").await);
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
            return Err(transport_status(response, "ingest").await);
        }
        Ok(())
    }
}

async fn transport_status(response: reqwest::Response, what: &str) -> FederationError {
    let status = response.status();
    let snippet = error_body_snippet(response).await;
    if snippet.is_empty() {
        FederationError::Transport(format!("remote {what} returned {status}"))
    } else {
        FederationError::Transport(format!("remote {what} returned {status}: {snippet}"))
    }
}

/// At most 512 bytes, and at most eight chunks. The rest of a rejected body is
/// discarded with the response.
async fn error_body_snippet(mut resp: reqwest::Response) -> String {
    let mut buf = Vec::with_capacity(512);
    for _ in 0..8 {
        if buf.len() >= 512 {
            break;
        }
        match resp.chunk().await {
            Ok(Some(chunk)) if !chunk.is_empty() => {
                let take = (512 - buf.len()).min(chunk.len());
                buf.extend_from_slice(&chunk[..take]);
            }
            _ => break,
        }
    }
    drop(resp);
    String::from_utf8_lossy(&buf).trim().to_string()
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
