//! Background poll loop that pulls events from enabled federation peers.

use std::time::Duration;

use maidan_a2a::{FederationEnvelope, Outbound};
use maidan_types::Peer;
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::federation::{ingest_envelope, poll_interval_secs_from_env, resolve_outbound_secret};
use crate::state::AppState;

pub struct FederationWorker {
    shutdown: watch::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl FederationWorker {
    pub fn spawn(state: AppState) -> Self {
        let interval = Duration::from_secs(poll_interval_secs_from_env());
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let handle = tokio::spawn(run(state, interval, shutdown_rx));
        Self {
            shutdown: shutdown_tx,
            handle,
        }
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.handle.await;
    }
}

async fn run(state: AppState, interval: Duration, mut shutdown: watch::Receiver<()>) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(interval) => {
                if let Err(err) = poll_once(&state).await {
                    warn!(error = %err, "federation poll tick failed");
                }
            }
        }
    }
}

/// One pass over every enabled peer. A peer that fails is logged, counted and
/// skipped; the others are still polled. Returning early on the first failure
/// let one unreachable peer stop federation for every peer sorted after it, in
/// any workspace, on every tick.
pub async fn poll_once(state: &AppState) -> Result<(), String> {
    let peers = state
        .store
        .list_enabled_peers()
        .await
        .map_err(|e| e.to_string())?;
    for peer in peers {
        // A workspace past its deliveries allowance polls none of its peers
        // until the cutoff lifts. Its cursor stays where it is, so the next
        // poll after that picks up from the same place.
        let outcome = if state
            .usage_meters
            .is_cut_off(peer.workspace_id, maidan_types::UsageMeter::Deliveries)
        {
            tracing::info!(
                peer = %peer.id,
                workspace_id = %peer.workspace_id.0,
                reason = "over_allowance",
                "federation poll skipped; workspace past its free allowance for deliveries"
            );
            PollOutcome::OverAllowance
        } else {
            poll_peer(state, &peer).await
        };
        // No peer or workspace label: /metrics is instance-wide, and the
        // per-peer detail is already in the logs.
        metrics::counter!(
            "maidan_federation_peer_polls_total",
            "outcome" => outcome.as_str()
        )
        .increment(1);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PollOutcome {
    Ok,
    Refused,
    NoSecret,
    Failed,
    OverAllowance,
}

impl PollOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Refused => "refused",
            Self::NoSecret => "no_secret",
            Self::Failed => "failed",
            Self::OverAllowance => "over_allowance",
        }
    }
}

async fn poll_peer(state: &AppState, peer: &Peer) -> PollOutcome {
    let (client, _) = match crate::egress_http::client_for(&peer.base_url).await {
        Ok(target) => target,
        Err(err) => {
            warn!(peer = %peer.id, error = %err, "federation peer target refused");
            return PollOutcome::Refused;
        }
    };
    let outbound = Outbound::new(client);
    let Some(secret) = resolve_outbound_secret(state, peer) else {
        warn!(
            peer = %peer.id,
            "skipping poll: no outbound secret (set FEDERATION_ENCRYPTION_KEY and re-create peer)"
        );
        return PollOutcome::NoSecret;
    };
    let listed = outbound
        .list_events(peer, &secret, peer.last_synced_event_id, 100)
        .await;
    // Each poll is one outbound request, counted whether or not it succeeds.
    state
        .usage_meters
        .record(peer.workspace_id, maidan_types::UsageMeter::Deliveries, 1);
    let events = match listed {
        Ok(events) => events,
        Err(err) => {
            warn!(peer = %peer.id, error = %err, "federation poll failed; trying again next tick");
            return PollOutcome::Failed;
        }
    };
    // The cursor advances only over events actually processed. It used to
    // be set to the batch's highest id regardless of outcome, so a failed
    // ingest was skipped past and never retried — silent, permanent loss. A
    // failure now stops the batch and leaves the cursor behind it, so the
    // next poll retries from the same place.
    let mut max_id = peer.last_synced_event_id;
    for stored in events {
        let remote_event_id = stored.id;
        let envelope = FederationEnvelope {
            origin_peer_id: peer.id,
            remote_event_id,
            event: stored,
        };
        match ingest_envelope(state, peer, envelope).await {
            Ok(_) => max_id = max_id.max(remote_event_id),
            Err(err) => {
                warn!(
                    peer = %peer.id,
                    remote_event_id,
                    error = ?err,
                    "ingest failed; holding the cursor so the next poll retries"
                );
                break;
            }
        }
    }
    if max_id > peer.last_synced_event_id {
        if let Err(err) = state.store.update_peer_cursor(peer.id, max_id).await {
            warn!(peer = %peer.id, error = %err, "could not save the federation cursor");
            return PollOutcome::Failed;
        }
        debug!(peer = %peer.id, cursor = max_id, "advanced federation cursor");
    }
    PollOutcome::Ok
}
