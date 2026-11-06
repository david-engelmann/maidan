//! `GET /operator/status` — one page an operator reads before a deploy or during
//! an incident: what phase the server is in, whether search has caught up with
//! the event log, how far a read replica lags, and how full the queues are.
//!
//! JSON by default; `Accept: text/html` gets the same facts as a page with no
//! scripts. Instance-wide, so `operator:global` — the search tap and the queues
//! span every workspace.

use std::sync::atomic::Ordering;
use std::sync::LazyLock;

use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::{Html, IntoResponse, Response},
    Extension, Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use maidan_auth::{capability::OPERATOR_GLOBAL, AuthContext};
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::health::{collect, SubsystemStatus};
use crate::routes::{cap, ApiResult};
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct OperatorStatus {
    pub version: &'static str,
    /// `serving`, `degraded` (a readiness check fails) or `draining` (SIGTERM
    /// received; the listener closes after the drain delay).
    pub phase: &'static str,
    pub checks: Vec<StatusCheck>,
    pub search: SearchProgress,
    /// `None` without a read replica.
    pub replica: Option<ReplicaStatus>,
    pub queues: QueueStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatusCheck {
    pub name: &'static str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchProgress {
    /// Highest event-log id on the instance.
    pub event_log_head: i64,
    /// Where the search tap last finished.
    pub tap_cursor: i64,
    /// Events the search tap has not projected yet.
    pub behind: i64,
    /// `tap_cursor` as a share of the head, `100.0` for an empty log. Below 100
    /// right after a rebuild means the backfill is still running.
    pub backfill_percent: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReplicaStatus {
    pub lag_bytes: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QueueStatus {
    pub indexer_depth: usize,
    pub indexer_capacity: usize,
    pub indexer_failed_total: u64,
    pub ws_connections: usize,
    pub ws_connections_max: usize,
    /// `None` when no transactional outbox is configured.
    pub outbox_pending: Option<i64>,
    pub outbox_quarantined: Option<i64>,
}

fn check(name: &'static str, status: &SubsystemStatus) -> StatusCheck {
    match status {
        SubsystemStatus::Ok => StatusCheck {
            name,
            ok: true,
            error: None,
        },
        SubsystemStatus::Error(err) => StatusCheck {
            name,
            ok: false,
            error: Some(err.clone()),
        },
    }
}

pub(crate) async fn gather(state: &AppState) -> ApiResult<OperatorStatus> {
    let health = collect(state).await;
    let phase = match health.status.as_str() {
        "ok" => "serving",
        "draining" => "draining",
        _ => "degraded",
    };
    let head = state.store.max_event_id().await?;
    let tap = state
        .store
        .tap_cursor(maidan_search::SEARCH_TAP_SURFACE)
        .await?;
    let backfill_percent = if head <= 0 {
        100.0
    } else {
        ((tap.min(head) as f64 / head as f64) * 1000.0).round() / 10.0
    };
    let (outbox_pending, outbox_quarantined) = match state.outbox_backend.as_ref() {
        Some(backend) => (
            crate::outbox_relay::pending_count(backend).await.ok(),
            crate::outbox_relay::quarantined_count(backend).await.ok(),
        ),
        None => (None, None),
    };
    let im = &state.indexer_metrics;
    Ok(OperatorStatus {
        version: crate::version(),
        phase,
        checks: vec![
            check("db", &health.db),
            check("storage", &health.storage),
            check("indexer", &health.indexer),
            check("bus", &health.bus),
        ],
        search: SearchProgress {
            event_log_head: head,
            tap_cursor: tap,
            behind: (head - tap).max(0),
            backfill_percent,
        },
        replica: state
            .read_routing_metrics
            .as_ref()
            .map(|routing| ReplicaStatus {
                lag_bytes: routing.lag_bytes(),
            }),
        queues: QueueStatus {
            indexer_depth: im.queue_depth.load(Ordering::Relaxed),
            indexer_capacity: im.queue_capacity,
            indexer_failed_total: im.failed_total.load(Ordering::Relaxed),
            ws_connections: state.ws_connections.load(Ordering::Relaxed),
            ws_connections_max: state.max_ws_connections,
            outbox_pending,
            outbox_quarantined,
        },
    })
}

pub async fn operator_status(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    cap(&auth, OPERATOR_GLOBAL)?;
    let status = gather(&state).await?;
    let wants_html = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"));
    Ok(if wants_html {
        (
            [
                (header::CONTENT_SECURITY_POLICY, html_csp()),
                (header::REFERRER_POLICY, "no-referrer"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            Html(render(&status)),
        )
            .into_response()
    } else {
        Json(status).into_response()
    })
}

const CSS: &str = r#"
:root{color-scheme:light dark;--paper:#f2eee6;--card:#faf8f3;--ink:#151411;--muted:#5c5850;--line:rgba(21,20,17,.12);--fail:#7a1a12;--fail-bg:#f8e6e1}
@media (prefers-color-scheme:dark){:root{--paper:#121110;--card:#1a1917;--ink:#e9e5dc;--muted:#9d988e;--line:rgba(233,229,220,.10);--fail:#ffd0c8;--fail-bg:#3b1c18}}
*{box-sizing:border-box}
html,body{margin:0;background:var(--paper);color:var(--ink)}
body{min-height:100vh;font:15px/1.5 ui-sans-serif,system-ui,-apple-system,sans-serif;padding:max(16px,env(safe-area-inset-top)) max(16px,env(safe-area-inset-right)) max(32px,env(safe-area-inset-bottom)) max(16px,env(safe-area-inset-left))}
main{max-width:44rem;margin:0 auto;background:var(--card);border:1px solid var(--line);border-radius:16px;padding:24px}
h1{font-size:22px;line-height:1.25;margin:0 0 16px;font-weight:600}
table{width:100%;border-collapse:collapse;table-layout:fixed}
th,td{padding:8px 0;vertical-align:top;overflow-wrap:anywhere;text-align:left}
th{width:38%;color:var(--muted);font-weight:600;padding-right:16px}
td{color:var(--ink)}
tr.phase td{font-weight:600}
tr.fail td{color:var(--fail);background:var(--fail-bg);font-weight:600;border-radius:8px;padding:8px 10px}
"#;

static CSP: LazyLock<String> = LazyLock::new(|| {
    let hash = STANDARD.encode(Sha256::digest(CSS.as_bytes()));
    format!(
        "default-src 'none'; style-src 'sha256-{hash}'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    )
});

/// The operator page's CSP: the inline style, and nothing else.
pub fn html_csp() -> &'static str {
    CSP.as_str()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn row(label: &str, value: &str) -> String {
    let class = if value.starts_with("failing:") {
        " class=\"fail\""
    } else if label == "Phase" {
        " class=\"phase\""
    } else {
        ""
    };
    format!("<tr{class}><th>{}</th><td>{}</td></tr>", esc(label), esc(value))
}

fn render(s: &OperatorStatus) -> String {
    let mut rows = vec![row("Version", s.version), row("Phase", s.phase)];
    for c in &s.checks {
        let value = match &c.error {
            None => "ok".to_string(),
            Some(err) => format!("failing: {err}"),
        };
        rows.push(row(c.name, &value));
    }
    rows.push(row(
        "Search backfill",
        &format!(
            "{}% ({} of {} events, {} behind)",
            s.search.backfill_percent,
            s.search.tap_cursor,
            s.search.event_log_head,
            s.search.behind
        ),
    ));
    rows.push(row(
        "Replica lag",
        &match &s.replica {
            Some(r) => format!("{} bytes", r.lag_bytes),
            None => "no replica".to_string(),
        },
    ));
    let q = &s.queues;
    rows.push(row(
        "Indexer queue",
        &format!(
            "{} / {} ({} failed)",
            q.indexer_depth, q.indexer_capacity, q.indexer_failed_total
        ),
    ));
    rows.push(row(
        "WebSocket connections",
        &format!("{} / {}", q.ws_connections, q.ws_connections_max),
    ));
    if let (Some(pending), Some(quarantined)) = (q.outbox_pending, q.outbox_quarantined) {
        rows.push(row(
            "Outbox",
            &format!("{pending} pending, {quarantined} quarantined"),
        ));
    }
    format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"><meta name="color-scheme" content="light dark"><meta name="theme-color" content="#f2eee6" media="(prefers-color-scheme: light)"><meta name="theme-color" content="#121110" media="(prefers-color-scheme: dark)"><meta name="referrer" content="no-referrer"><title>Maidan status</title><style>{CSS}</style></head><body><main><h1>Maidan status</h1><table>{rows}</table></main></body></html>"##,
        rows = rows.join("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sha2::{Digest, Sha256};

    #[test]
    fn the_page_escapes_what_it_reports() {
        let status = OperatorStatus {
            version: "v0",
            phase: "degraded",
            checks: vec![StatusCheck {
                name: "db",
                ok: false,
                error: Some("<script>x</script>".into()),
            }],
            search: SearchProgress {
                event_log_head: 10,
                tap_cursor: 5,
                behind: 5,
                backfill_percent: 50.0,
            },
            replica: None,
            queues: QueueStatus {
                indexer_depth: 0,
                indexer_capacity: 1,
                indexer_failed_total: 0,
                ws_connections: 0,
                ws_connections_max: 1,
                outbox_pending: None,
                outbox_quarantined: None,
            },
        };
        let page = render(&status);
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(page.contains("50% (5 of 10 events, 5 behind)"));
        assert!(page.contains("<th>Phase</th><td>degraded</td>"));
        assert!(page.contains("<main>"));
    }

    #[test]
    fn the_csp_pins_the_style_element_by_hash_and_allows_nothing_inline() {
        let page = render(&OperatorStatus {
            version: "v0",
            phase: "serving",
            checks: vec![],
            search: SearchProgress {
                event_log_head: 0,
                tap_cursor: 0,
                behind: 0,
                backfill_percent: 100.0,
            },
            replica: None,
            queues: QueueStatus {
                indexer_depth: 0,
                indexer_capacity: 1,
                indexer_failed_total: 0,
                ws_connections: 0,
                ws_connections_max: 1,
                outbox_pending: None,
                outbox_quarantined: None,
            },
        });
        let open = "<style>";
        let start = page.find(open).expect("style") + open.len();
        let end = page[start..].find("</style>").expect("style end") + start;
        let inside = &page[start..end];
        assert_eq!(inside, CSS);
        assert!(page.contains(&format!("<style>{CSS}</style>")));
        let hash = STANDARD.encode(Sha256::digest(inside.as_bytes()));
        let csp = html_csp();
        assert!(
            csp.contains(&format!("style-src 'sha256-{hash}'")),
            "{csp}"
        );
        assert!(!csp.contains("unsafe"));
        assert!(csp.starts_with("default-src 'none'"));
    }
}
