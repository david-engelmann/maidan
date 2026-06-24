# Cluster 130.0 — Test-coverage uplift (observability + MCP)

**Theme:** Add deterministic tests to two of the thinnest-covered areas from the
v126 hardening scan — observability env-parsing (zero tests) and an untested MCP
module — via pure-function refactors that avoid env/DB flakiness.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v130.0.0`**, no new
gate tag. Final core cluster of the hardening sweep (127 → 128 → 129 → **130**).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **observability** | Extract pure parsers (`is_truthy`, `resolve_metrics_endpoint`, `parse_metrics_interval`, `parse_log_format`); the `*_from_env` wrappers feed `std::env::var` into them. 6 unit tests, no env mutation. |
| **maidan-mcp** | `prompts.rs` catalog-integrity test (previously untested module). |

## Non-goals

- DB-backed tool-handler tests (covered by server e2e) — this cluster targets
  pure logic with no DB/env dependency.
- Chasing a coverage *number*; the 40% floor is already enforced. This fills
  specific zero-coverage gaps the scan named.

## Why pure refactors

Testing `from_env()` directly means `std::env::set_var` in a parallel test
binary — a data race with any concurrent `env::var` read. Extracting the logic
into pure functions makes the tests deterministic and race-free, and is cleaner
code besides.

## PR ladder (actual)

| # | Title |
|---|--------|
| 130.0.1 | `test: cover observability env-parsing + MCP prompts catalog` (#351) |
| 130.0.retro | `docs(retro): Cluster 130.0 + v130.0.0 tag prep` |

## Exit criteria

- Observability env-parsing + MCP prompts have unit tests — **met**.
- `v130.0.0` tagged after retro.

## References

- [[Retros/Cluster 130.0]]; v126 hardening scan
- `crates/maidan-observability/src/{metrics,lib}.rs`, `crates/maidan-mcp/src/prompts.rs`
