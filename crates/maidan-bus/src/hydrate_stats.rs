//! Cumulative hydrate outcomes for Postgres NOTIFY pointer delivery.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrateResult {
    Ok,
    NotFound,
    Failed,
    InvalidPayload,
}

#[derive(Debug, Default)]
pub struct HydrateStats {
    ok: AtomicU64,
    not_found: AtomicU64,
    failed: AtomicU64,
    invalid_payload: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HydrateSnapshot {
    pub ok: u64,
    pub not_found: u64,
    pub failed: u64,
    pub invalid_payload: u64,
}

impl HydrateStats {
    pub fn record(&self, result: HydrateResult) {
        let counter = match result {
            HydrateResult::Ok => &self.ok,
            HydrateResult::NotFound => &self.not_found,
            HydrateResult::Failed => &self.failed,
            HydrateResult::InvalidPayload => &self.invalid_payload,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> HydrateSnapshot {
        HydrateSnapshot {
            ok: self.ok.load(Ordering::Relaxed),
            not_found: self.not_found.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            invalid_payload: self.invalid_payload.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_increments_snapshot() {
        let stats = HydrateStats::default();
        stats.record(HydrateResult::NotFound);
        stats.record(HydrateResult::Ok);
        let snap = stats.snapshot();
        assert_eq!(snap.not_found, 1);
        assert_eq!(snap.ok, 1);
    }

    #[test]
    fn all_result_labels_accumulate() {
        let stats = HydrateStats::default();
        stats.record(HydrateResult::Ok);
        stats.record(HydrateResult::NotFound);
        stats.record(HydrateResult::Failed);
        stats.record(HydrateResult::InvalidPayload);
        let snap = stats.snapshot();
        assert_eq!(snap.ok, 1);
        assert_eq!(snap.not_found, 1);
        assert_eq!(snap.failed, 1);
        assert_eq!(snap.invalid_payload, 1);
    }
}
