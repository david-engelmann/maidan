# Store bench baseline (Cluster 120.0, gate `maidan-scale-1.0`)

Run:

```sh
cargo bench -p maidan-store --bench store_hot
```

`store_hot.rs` measures a hot read path (`list_members`, 32 members) on an
in-memory SQLite store. SQLite keeps it self-contained (no testcontainer) and
reproducible in CI.

These numbers are **machine-specific** — treat them as a relative reference for
the Cluster 120 perf budgets, not an absolute SLA. Re-run on the target
hardware to establish the local floor.

## Reference run (Apple Silicon dev laptop, release profile, criterion 100 samples)

| Bench | Median |
|-------|--------|
| `sqlite_list_members_32` | ~0.10 ms (≈101 µs) |

## Postgres

Postgres store latency depends on connection-pool sizing (Cluster 107) and the
statement-timeout cap, and must be measured against a real instance with
representative data volume. This SQLite bench is the CI-friendly floor; see the
gate doc ([`docs/Gates/maidan-scale-1.0.md`](../../../docs/Gates/maidan-scale-1.0.md))
for how the perf budget is recorded.
