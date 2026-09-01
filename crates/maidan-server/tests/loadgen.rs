//! Cluster 198 (Arc D, part 1): an on-demand load / soak harness.
//!
//! Arc D optimizes performance & scale — sharded fan-out, filtered-ANN search,
//! batched context assembly. Those need a *baseline*: a repeatable way to drive
//! concurrent traffic at the server and report latency percentiles + throughput,
//! so an optimization can be shown to move the number.
//!
//! `load_baseline` is `#[ignore]`d — it is a measurement tool, not a pass/fail
//! CI gate (a hard latency floor would flake across runner hardware). Run it
//! explicitly:
//!
//! ```sh
//! # in-process server (SQLite), 8 workers × 50 iterations:
//! cargo test --release -p maidan-server --test loadgen -- --ignored --nocapture
//!
//! # tune via env, or point at a live/scaled deployment:
//! MAIDAN_LOADGEN_CONCURRENCY=32 MAIDAN_LOADGEN_DURATION_SECS=30 \
//!   MAIDAN_LOADGEN_URL=http://localhost:8080 MAIDAN_LOADGEN_BEARER=... \
//!   cargo test --release -p maidan-server --test loadgen -- --ignored --nocapture
//! ```
//!
//! `scripts/loadgen.sh` wraps the env knobs. The percentile math is pure and
//! unit-tested here (that part *does* run in CI).

use std::{
    env,
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::{Duration, Instant},
};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

/// Latency summary for one operation kind, in milliseconds.
#[derive(Debug, Clone, PartialEq)]
struct Stats {
    count: usize,
    min: f64,
    mean: f64,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

/// Nearest-rank percentiles over latency samples (ms). `None` when empty.
fn stats(mut samples: Vec<f64>) -> Option<Stats> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = samples.len();
    // Nearest-rank: the smallest value whose rank ≥ p% of n (rank is 1-based).
    let pct = |p: f64| -> f64 {
        let rank = ((p / 100.0) * n as f64).ceil() as usize;
        let idx = rank.saturating_sub(1).min(n - 1);
        samples[idx]
    };
    let sum: f64 = samples.iter().sum();
    Some(Stats {
        count: n,
        min: samples[0],
        mean: sum / n as f64,
        p50: pct(50.0),
        p95: pct(95.0),
        p99: pct(99.0),
        max: samples[n - 1],
    })
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Derive the WebSocket base from an HTTP base URL (`http`→`ws`, `https`→`wss`).
fn http_to_ws(base: &str) -> String {
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    }
}

struct InProcess {
    _server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
    base: String,
    bearer: String,
}

/// Spin up an in-process server (auth ENABLED, SQLite) seeded with a workspace,
/// channel, thread, and a full-capability token — the default target when
/// `MAIDAN_LOADGEN_URL` is unset.
async fn spawn_in_process() -> (InProcess, String) {
    // Match the shipped SQLite default (Cluster 277): one connection. A
    // multi-connection SQLite pool deadlocks under write contention, so
    // benchmarking 16 connections would measure a configuration Maidan does not
    // ship. `min_connections(1)` keeps the single `sqlite::memory:` connection
    // (and its in-memory database) alive for the whole run.
    let pool = SqlitePoolOptions::new()
        .max_connections(maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS)
        .min_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));

    let ws = store
        .create_workspace(NewWorkspace {
            name: "loadgen".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "driver".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let bearer = mint(store.as_ref(), ws.id, member.id).await;

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    // A channel + thread to drive traffic against.
    let client = reqwest::Client::new();
    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{}/channels", ws.id.0))
        .bearer_auth(&bearer)
        .json(&json!({"name": "load"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: serde_json::Value = client
        .post(format!("{base}/channels/{cid}/threads"))
        .bearer_auth(&bearer)
        .json(&json!({"title": "load"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();

    (
        InProcess {
            _server: server,
            _dir: dir,
            base: base.clone(),
            bearer: bearer.clone(),
        },
        format!(
            "{ws_id}|{cid}|{tid}|{mid}",
            ws_id = ws.id.0,
            mid = member.id.0
        ),
    )
}

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::SEARCH_QUERY.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

/// One worker's per-op latency samples (ms), keyed by op name.
#[derive(Default)]
struct WorkerResult {
    post: Vec<f64>,
    read: Vec<f64>,
    search: Vec<f64>,
    errors: u64,
}

async fn timed<F, Fut>(samples: &mut Vec<f64>, errors: &mut u64, f: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let start = Instant::now();
    match f().await {
        Ok(resp) if resp.status().is_success() => {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        _ => *errors += 1,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "load/soak measurement tool — run explicitly with --ignored"]
async fn load_baseline() {
    let concurrency = env_u64("MAIDAN_LOADGEN_CONCURRENCY", 8).max(1);
    let ops = env_u64("MAIDAN_LOADGEN_OPS", 50).max(1);
    let duration_secs = env_u64("MAIDAN_LOADGEN_DURATION_SECS", 0);
    let deadline = (duration_secs > 0).then(|| Instant::now() + Duration::from_secs(duration_secs));

    // Target: an external deployment, or an in-process server.
    let external_url = env::var("MAIDAN_LOADGEN_URL").ok();
    let (_owned, base, bearer, ids) = match &external_url {
        Some(url) => {
            let bearer = env::var("MAIDAN_LOADGEN_BEARER").unwrap_or_default();
            let ids = env::var("MAIDAN_LOADGEN_IDS").expect(
                "MAIDAN_LOADGEN_IDS=workspace|channel|thread|member required when targeting a URL",
            );
            (None, url.clone(), bearer, ids)
        }
        None => {
            let (proc, ids) = spawn_in_process().await;
            let (base, bearer) = (proc.base.clone(), proc.bearer.clone());
            (Some(proc), base, bearer, ids)
        }
    };
    let parts: Vec<&str> = ids.split('|').collect();
    assert!(
        parts.len() >= 4,
        "MAIDAN_LOADGEN_IDS must be workspace|channel|thread|member"
    );
    let (ws_id, _cid, tid, mid) = (
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
        parts[3].to_string(),
    );

    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(concurrency as usize)
        .build()
        .unwrap();

    let mode = match deadline {
        Some(_) => format!("soak {duration_secs}s"),
        None => format!("{ops} iters/worker"),
    };
    eprintln!("loadgen: target={base} concurrency={concurrency} {mode}");

    let wall = Instant::now();
    let mut set = tokio::task::JoinSet::new();
    for w in 0..concurrency {
        let client = client.clone();
        let (base, bearer, ws_id, tid, mid) = (
            base.clone(),
            bearer.clone(),
            ws_id.clone(),
            tid.clone(),
            mid.clone(),
        );
        set.spawn(async move {
            let mut r = WorkerResult::default();
            let mut i = 0u64;
            loop {
                match deadline {
                    Some(dl) if Instant::now() >= dl => break,
                    None if i >= ops => break,
                    _ => {}
                }
                // post a message
                timed(&mut r.post, &mut r.errors, || {
                    client
                        .post(format!("{base}/threads/{tid}/messages"))
                        .bearer_auth(&bearer)
                        .json(&json!({
                            "author_id": mid,
                            "body": format!("worker {w} op {i}")
                        }))
                        .send()
                })
                .await;
                // read the thread's messages
                timed(&mut r.read, &mut r.errors, || {
                    client
                        .get(format!("{base}/threads/{tid}/messages?limit=50"))
                        .bearer_auth(&bearer)
                        .send()
                })
                .await;
                // search the workspace
                timed(&mut r.search, &mut r.errors, || {
                    client
                        .get(format!(
                            "{base}/workspaces/{ws_id}/search?q=worker&limit=10"
                        ))
                        .bearer_auth(&bearer)
                        .send()
                })
                .await;
                i += 1;
            }
            r
        });
    }

    let mut post = Vec::new();
    let mut read = Vec::new();
    let mut search = Vec::new();
    let mut errors = 0u64;
    while let Some(res) = set.join_next().await {
        let r = res.unwrap();
        post.extend(r.post);
        read.extend(r.read);
        search.extend(r.search);
        errors += r.errors;
    }
    let elapsed = wall.elapsed().as_secs_f64();
    let total = post.len() + read.len() + search.len();

    eprintln!(
        "\n=== loadgen report ({total} ok ops, {errors} errors, {elapsed:.2}s, {:.0} ops/s) ===",
        total as f64 / elapsed.max(f64::MIN_POSITIVE)
    );
    eprintln!(
        "{:<8} {:>7} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "op", "count", "min", "mean", "p50", "p95", "p99", "max"
    );
    for (name, s) in [("post", post), ("read", read), ("search", search)] {
        if let Some(st) = stats(s) {
            eprintln!(
                "{name:<8} {:>7} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.2}",
                st.count, st.min, st.mean, st.p50, st.p95, st.p99, st.max
            );
        }
    }
    // A run that produced traffic should have measured at least one op.
    assert!(total > 0, "load run produced no successful operations");
}

/// Measure **post→observer latency**: the time from a producer initiating a
/// message post to a subscribed observer receiving that message over the
/// realtime WebSocket. This is the propagation number a real-time claim rests
/// on (distinct from `load_baseline`'s per-op REST latency + throughput).
///
/// Each iteration posts a uniquely-tagged message and, *concurrently* with the
/// POST, reads WebSocket frames until the matching event arrives — so the
/// sample reflects true fan-out, not the POST round-trip. Serial (one message
/// in flight) to keep correlation unambiguous. Same target selection as
/// `load_baseline` (in-process SQLite by default; `MAIDAN_LOADGEN_URL` +
/// `_BEARER` + `_IDS` for an external deployment).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "post→observer latency measurement — run explicitly with --ignored"]
async fn post_to_observer_latency() {
    let ops = env_u64("MAIDAN_LOADGEN_OBSERVER_OPS", 200).max(1);

    let external_url = env::var("MAIDAN_LOADGEN_URL").ok();
    let (_owned, base, bearer, ids) = match &external_url {
        Some(url) => {
            let bearer = env::var("MAIDAN_LOADGEN_BEARER").unwrap_or_default();
            let ids = env::var("MAIDAN_LOADGEN_IDS").expect(
                "MAIDAN_LOADGEN_IDS=workspace|channel|thread|member required when targeting a URL",
            );
            (None, url.clone(), bearer, ids)
        }
        None => {
            let (proc, ids) = spawn_in_process().await;
            let (base, bearer) = (proc.base.clone(), proc.bearer.clone());
            (Some(proc), base, bearer, ids)
        }
    };
    let parts: Vec<&str> = ids.split('|').collect();
    assert!(
        parts.len() >= 4,
        "MAIDAN_LOADGEN_IDS must be workspace|channel|thread|member"
    );
    let (tid, mid) = (parts[2].to_string(), parts[3].to_string());

    // Connect the observer. `/ws/subscribe` reads the bearer from the subscribe
    // frame's `token` field, not an HTTP header, so the upgrade itself is plain.
    let ws_url = format!("{}/ws/subscribe", http_to_ws(&base));
    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("observer ws connect");

    // Authenticate + subscribe in one frame. Correlation is by a per-op nonce in
    // the body, so a kind-only filter is sufficient (independent of the thread
    // filter). Wait for the ack so the bus subscription is attached before posting.
    ws.send(Message::Text(
        json!({"filter": {"kinds": ["message_posted"]}, "token": bearer}).to_string(),
    ))
    .await
    .unwrap();
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(p))) => {
                let v: serde_json::Value =
                    serde_json::from_str(&p).unwrap_or(serde_json::Value::Null);
                if v.get("type").and_then(|t| t.as_str()) == Some("subscribe_ack") {
                    break;
                }
            }
            Some(Ok(_)) => {}
            other => panic!("observer ws closed before subscribe_ack: {other:?}"),
        }
    }

    let client = reqwest::Client::new();
    let mut samples: Vec<f64> = Vec::with_capacity(ops as usize);
    let mut errors = 0u64;

    let wall = Instant::now();
    for seq in 0..ops {
        let nonce = format!(
            "obsbench-{seq}-{:x}",
            (seq.wrapping_mul(2654435761)) & 0xffffff
        );
        let t0 = Instant::now();
        let post = client
            .post(format!("{base}/threads/{tid}/messages"))
            .bearer_auth(&bearer)
            .json(&json!({"author_id": mid, "body": nonce}))
            .send();
        // Read frames until the matching event, concurrently with the POST, so
        // t1 records receipt independent of when the POST response returns.
        let observe = async {
            loop {
                match ws.next().await {
                    Some(Ok(Message::Text(p))) => {
                        let v: serde_json::Value = match serde_json::from_str(&p) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        if v.get("type").is_some() {
                            continue; // control frame
                        }
                        let body = v
                            .get("message")
                            .and_then(|m| m.get("body"))
                            .and_then(|b| b.as_str());
                        if body == Some(nonce.as_str()) {
                            return Some(Instant::now());
                        }
                    }
                    Some(Ok(_)) => {}
                    _ => return None, // stream closed
                }
            }
        };
        let (post_res, observed) = tokio::join!(post, observe);
        match (post_res, observed) {
            (Ok(r), Some(t1)) if r.status().is_success() => {
                samples.push((t1 - t0).as_secs_f64() * 1000.0);
            }
            _ => errors += 1,
        }
    }
    let elapsed = wall.elapsed().as_secs_f64();

    let n = samples.len();
    eprintln!("\n=== post→observer latency ({n} samples, {errors} errors, {elapsed:.2}s) ===",);
    eprintln!(
        "{:<14} {:>7} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "metric", "count", "min", "mean", "p50", "p95", "p99", "max"
    );
    if let Some(st) = stats(samples) {
        eprintln!(
            "{:<14} {:>7} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.2}",
            "post->observer", st.count, st.min, st.mean, st.p50, st.p95, st.p99, st.max
        );
    }
    assert!(n > 0, "no post→observer samples measured");
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles_over_1_to_100() {
        let s: Vec<f64> = (1..=100).map(|v| v as f64).collect();
        let st = stats(s).unwrap();
        assert_eq!(st.count, 100);
        assert_eq!(st.min, 1.0);
        assert_eq!(st.max, 100.0);
        assert_eq!(st.p50, 50.0);
        assert_eq!(st.p95, 95.0);
        assert_eq!(st.p99, 99.0);
        assert!((st.mean - 50.5).abs() < 1e-9);
    }

    #[test]
    fn empty_samples_have_no_stats() {
        assert!(stats(vec![]).is_none());
    }

    #[test]
    fn single_sample_is_every_percentile() {
        let st = stats(vec![7.0]).unwrap();
        assert_eq!(st.min, 7.0);
        assert_eq!(st.p50, 7.0);
        assert_eq!(st.p99, 7.0);
        assert_eq!(st.max, 7.0);
    }

    #[test]
    fn http_to_ws_maps_schemes() {
        assert_eq!(http_to_ws("http://127.0.0.1:8080"), "ws://127.0.0.1:8080");
        assert_eq!(http_to_ws("https://maidan.example"), "wss://maidan.example");
        // Leave an already-ws or unknown scheme untouched.
        assert_eq!(http_to_ws("ws://host"), "ws://host");
    }
}
