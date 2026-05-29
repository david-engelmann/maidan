//! Invokes registered FSM hooks when `ThreadStateChanged` events appear on the bus.

use std::time::Duration;

use maidan_bus::{BusItem, EventStream};
use maidan_types::{Event, EventFilter, EventKind};
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tracing::{info, warn};

use crate::fsm_hooks::dispatch_thread_state_changed;
use crate::state::AppState;

const RECONNECT_INITIAL: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(5);

pub struct FsmHookWorker {
    shutdown: watch::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl FsmHookWorker {
    pub fn spawn(state: AppState) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let handle = tokio::spawn(run(state, shutdown_rx));
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

async fn run(state: AppState, mut shutdown: watch::Receiver<()>) {
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    let stop_forward = stop_tx.clone();
    tokio::spawn(async move {
        let _ = shutdown.changed().await;
        let _ = stop_forward.send(()).await;
    });

    let mut backoff = RECONNECT_INITIAL;
    loop {
        let filter = EventFilter::all().with_kinds([EventKind::ThreadStateChanged]);
        let stream = match state.bus.subscribe(filter).await {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, ?backoff, "fsm hook bus subscribe failed; retrying");
                if tokio::time::timeout(backoff, stop_rx.recv()).await.is_ok() {
                    return;
                }
                backoff = (backoff * 2).min(RECONNECT_MAX);
                continue;
            }
        };
        backoff = RECONNECT_INITIAL;
        info!("fsm hook worker attached to bus");
        if consume(stream, &state, &mut stop_rx).await {
            return;
        }
        warn!("fsm hook bus stream ended; resubscribing");
    }
}

async fn consume(
    mut stream: EventStream,
    state: &AppState,
    stop_rx: &mut mpsc::Receiver<()>,
) -> bool {
    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(BusItem::Event(envelope)) => {
                        if let Event::ThreadStateChanged {
                            workspace_id,
                            channel_id,
                            thread_id,
                            actor_id,
                            from_state,
                            to_state,
                            thread,
                            ..
                        } = envelope.event
                        {
                            dispatch_thread_state_changed(
                                state,
                                workspace_id,
                                channel_id,
                                thread_id,
                                actor_id,
                                from_state,
                                to_state,
                                thread,
                            )
                            .await;
                        }
                    }
                    Some(BusItem::Lagged { skipped }) => {
                        warn!(skipped, "fsm hook bus subscriber lagged");
                    }
                    None => return false,
                }
            }
            _ = stop_rx.recv() => {
                info!("fsm hook worker shutdown");
                return true;
            }
        }
    }
}
