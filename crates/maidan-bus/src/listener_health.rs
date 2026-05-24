//! Health signals for the Postgres `LISTEN` background task.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

#[derive(Debug, Default)]
pub struct ListenerHealth {
    degraded: AtomicBool,
    last_ok_unix_ms: AtomicI64,
    last_error_unix_ms: AtomicI64,
}

impl ListenerHealth {
    pub fn record_ok(&self) {
        self.degraded.store(false, Ordering::Release);
        self.last_ok_unix_ms.store(now_unix_ms(), Ordering::Relaxed);
    }

    pub fn record_error(&self) {
        self.degraded.store(true, Ordering::Release);
        self.last_error_unix_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }

    pub fn check(&self) -> Result<(), String> {
        if self.degraded.load(Ordering::Acquire) {
            let ms = self.last_error_unix_ms.load(Ordering::Relaxed);
            return Err(format!(
                "postgres LISTEN listener degraded (last error unix_ms={ms})"
            ));
        }
        Ok(())
    }

    pub fn last_ok_unix_ms(&self) -> i64 {
        self.last_ok_unix_ms.load(Ordering::Relaxed)
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_ok_until_error_then_recovery() {
        let h = ListenerHealth::default();
        h.check().expect("initially ok");
        h.record_error();
        h.check().expect_err("degraded after error");
        h.record_ok();
        h.check().expect("ok after successful recv");
        assert!(h.last_ok_unix_ms() > 0);
    }
}
