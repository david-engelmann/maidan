//! Prometheus metrics for WebSocket and MCP SSE subscribe recovery paths.

use metrics::{counter, histogram};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeTransport {
    Ws,
    McpSse,
}

impl SubscribeTransport {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ws => "ws",
            Self::McpSse => "mcp_sse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeReplayOutcome {
    AutoReplay,
    ReplayHint,
    ReplayTruncated,
    AutoReplayFailed,
}

impl SubscribeReplayOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AutoReplay => "auto_replay",
            Self::ReplayHint => "replay_hint",
            Self::ReplayTruncated => "replay_truncated",
            Self::AutoReplayFailed => "auto_replay_failed",
        }
    }
}

pub fn record_bus_lag(transport: SubscribeTransport, skipped: u64) {
    counter!(
        "maidan_bus_lag_total",
        "transport" => transport.as_str()
    )
    .increment(1);
    histogram!(
        "maidan_bus_lag_skipped",
        "transport" => transport.as_str()
    )
    .record(skipped as f64);
}

pub fn record_subscribe_replay(transport: SubscribeTransport, outcome: SubscribeReplayOutcome) {
    counter!(
        "maidan_subscribe_replay_total",
        "transport" => transport.as_str(),
        "outcome" => outcome.as_str()
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use super::*;

    static INIT: Once = Once::new();

    fn init_metrics() {
        INIT.call_once(|| {
            crate::metrics::init();
        });
    }

    #[test]
    fn record_bus_lag_and_replay_do_not_panic() {
        init_metrics();
        record_bus_lag(SubscribeTransport::Ws, 3);
        record_subscribe_replay(
            SubscribeTransport::McpSse,
            SubscribeReplayOutcome::ReplayHint,
        );
    }
}
