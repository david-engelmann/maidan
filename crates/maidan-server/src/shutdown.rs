//! The process-wide signal that ends long-lived streams when the server stops.
//!
//! `axum::serve(..).with_graceful_shutdown` closes the listener on SIGTERM and
//! then waits for every open connection to finish. A server-sent event stream,
//! an MCP notification stream or a WebSocket never finishes on its own, so one
//! connected agent kept a stopped server's process alive for minutes. On
//! Cloudflare Containers the replacement starts only after the old process
//! exits, so each hosted deploy was down for as long as the longest stream
//! stayed open (3 to 5 minutes, Open Work row 47). Streams watch this signal
//! and end with a reason the client reconnects on, and [`SHUTDOWN_GRACE`]
//! bounds whatever else is still in flight.

use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::watch;

/// The reason a stream ended by shutdown gives: the SSE `stream_ended`
/// event's `reason`, and the WebSocket close frame's.
pub const SERVER_RESTARTING: &str = "server restarting";

/// How long requests still in flight get once the listener has closed. Past
/// it the server stops waiting for them and exits.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

fn signal() -> &'static watch::Sender<bool> {
    static SIGNAL: OnceLock<watch::Sender<bool>> = OnceLock::new();
    SIGNAL.get_or_init(|| watch::channel(false).0)
}

/// Begin shutdown: every stream holding a [`subscribe`] receiver ends.
pub fn begin() {
    signal().send_replace(true);
}

/// A receiver that reads `true` once shutdown has begun, including when it
/// began before the receiver was made.
pub fn subscribe() -> watch::Receiver<bool> {
    signal().subscribe()
}

/// Resolves once `shutdown` reads `true`. A receiver whose sender is gone
/// never resolves: nothing can begin that shutdown any more.
pub async fn begun(shutdown: &mut watch::Receiver<bool>) {
    let fired = shutdown.wait_for(|down| *down).await.is_ok();
    if !fired {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn begun_waits_for_the_signal_and_sees_one_already_sent() {
        let (tx, mut rx) = watch::channel(false);
        let wait = tokio::time::timeout(Duration::from_millis(20), begun(&mut rx)).await;
        assert!(wait.is_err(), "resolved before shutdown began");
        tx.send_replace(true);
        tokio::time::timeout(Duration::from_secs(1), begun(&mut rx))
            .await
            .expect("resolves once shutdown began");
        // A receiver made after the signal sees it too.
        let mut late = tx.subscribe();
        tokio::time::timeout(Duration::from_secs(1), begun(&mut late))
            .await
            .expect("a late receiver sees the signal");
    }

    #[tokio::test]
    async fn a_dropped_sender_never_reads_as_shutdown() {
        let (tx, mut rx) = watch::channel(false);
        drop(tx);
        let wait = tokio::time::timeout(Duration::from_millis(20), begun(&mut rx)).await;
        assert!(wait.is_err(), "a closed channel must not end streams");
    }
}
