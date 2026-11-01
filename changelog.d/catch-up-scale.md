### Catch-up verified flat as the event log grows

- **Added:** `crates/maidan-store/tests/catch_up_scale.rs` (UI deep dive P2, thread 18): catch-up pages through a 1,200-event workspace beside another workspace's events in clamped, ascending keyset pages with no event twice, and every query catch-up runs seeks an index rather than scanning the event table.
