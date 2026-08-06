//! Shared fixed-window rate limiter (in-memory or Redis).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug)]
pub struct WindowConfig {
    pub max: u32,
    pub window: Duration,
}

struct MemoryCounter {
    window_start: Instant,
    count: u32,
    window: Duration,
}

/// When the in-memory bucket map reaches this size, sweep entries whose window
/// has fully elapsed (Cluster 167). Without this the map grows without bound as
/// distinct keys — tokens/clients/routes × windows — accumulate: a memory leak.
const MEMORY_SWEEP_THRESHOLD: usize = 16_384;

static MEMORY_BUCKETS: LazyLock<Mutex<HashMap<String, MemoryCounter>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn memory_try_acquire(key: &str, cfg: WindowConfig) -> bool {
    let now = Instant::now();
    let mut buckets = MEMORY_BUCKETS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if buckets.len() >= MEMORY_SWEEP_THRESHOLD {
        buckets.retain(|_, e| now.duration_since(e.window_start) < e.window);
    }
    let entry = buckets.entry(key.to_string()).or_insert(MemoryCounter {
        window_start: now,
        count: 0,
        window: cfg.window,
    });
    entry.window = cfg.window;
    if now.duration_since(entry.window_start) >= cfg.window {
        entry.window_start = now;
        entry.count = 0;
    }
    if entry.count >= cfg.max {
        return false;
    }
    entry.count += 1;
    true
}

fn window_epoch_secs(window: Duration) -> u64 {
    let secs = window.as_secs().max(1);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / secs)
        .unwrap_or(0)
}

async fn redis_try_acquire(
    conn: &redis::aio::ConnectionManager,
    key: &str,
    cfg: WindowConfig,
) -> Option<bool> {
    let window_secs = cfg.window.as_secs().max(1);
    let epoch = window_epoch_secs(cfg.window);
    let redis_key = format!("maidan:rl:{key}:{epoch}");
    let mut c = conn.clone();
    let count: i64 = redis::cmd("INCR")
        .arg(&redis_key)
        .query_async(&mut c)
        .await
        .ok()?;
    if count == 1 {
        let _: Result<(), _> = redis::cmd("EXPIRE")
            .arg(&redis_key)
            .arg(window_secs)
            .query_async(&mut c)
            .await;
    }
    Some(count <= i64::from(cfg.max))
}

/// Returns `true` when the request is allowed under the fixed window.
pub async fn try_acquire(
    key: &str,
    cfg: WindowConfig,
    redis: Option<&redis::aio::ConnectionManager>,
) -> bool {
    if let Some(conn) = redis {
        if let Some(ok) = redis_try_acquire(conn, key, cfg).await {
            return ok;
        }
    }
    memory_try_acquire(key, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_window_resets() {
        let cfg = WindowConfig {
            max: 2,
            window: Duration::from_millis(50),
        };
        let key = "mem-test";
        assert!(try_acquire(key, cfg, None).await);
        assert!(try_acquire(key, cfg, None).await);
        assert!(!try_acquire(key, cfg, None).await);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(try_acquire(key, cfg, None).await);
    }

    #[test]
    fn memory_map_evicts_expired_windows_when_large() {
        // A very short window so every key's window elapses immediately, then
        // insert past the sweep threshold to trigger the retain (Cluster 167).
        let cfg = WindowConfig {
            max: 100,
            window: Duration::from_nanos(1),
        };
        for i in 0..(MEMORY_SWEEP_THRESHOLD + 5) {
            memory_try_acquire(&format!("evict-{i}"), cfg);
        }
        let len = MEMORY_BUCKETS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        assert!(
            len < MEMORY_SWEEP_THRESHOLD,
            "map must be bounded by eviction, got {len}"
        );
    }
}
