//! Background poll loop that pulls events from enabled federation peers.

use std::time::Duration;

use maidan_a2a::{FederationEnvelope, Outbound};
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
    let outbound = Outbound::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(interval) => {
                if let Err(err) = poll_once(&state, &outbound).await {
                    warn!(error = %err, "federation poll tick failed");
                }
            }
        }
    }
}

async fn poll_once(state: &AppState, outbound: &Outbound) -> Result<(), String> {
    let peers = state
        .store
        .list_enabled_peers()
        .await
        .map_err(|e| e.to_string())?;
    for peer in peers {
        let Some(secret) = resolve_outbound_secret(state, &peer) else {
            warn!(
                peer = %peer.id,
                "skipping poll: no outbound secret (set FEDERATION_ENCRYPTION_KEY and re-create peer)"
            );
            continue;
        };
        let events = outbound
            .list_events(&peer, &secret, peer.last_synced_event_id, 100)
            .await
            .map_err(|e| e.to_string())?;
        let mut max_id = peer.last_synced_event_id;
        for stored in events {
            max_id = max_id.max(stored.id);
            let envelope = FederationEnvelope {
                origin_peer_id: peer.id,
                remote_event_id: stored.id,
                event: stored,
            };
            match ingest_envelope(state, &peer, envelope).await {
                Ok(_) => {}
                Err(err) => warn!(peer = %peer.id, error = ?err, "ingest failed"),
            }
        }
        if max_id > peer.last_synced_event_id {
            let _ = state
                .store
                .update_peer_cursor(peer.id, max_id)
                .await
                .map_err(|e| e.to_string())?;
            debug!(peer = %peer.id, cursor = max_id, "advanced federation cursor");
        }
    }
    Ok(())
}
