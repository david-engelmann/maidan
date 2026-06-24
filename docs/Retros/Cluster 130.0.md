# Cluster 130.0 retro — Test-coverage uplift (observability + MCP)

> Tag **`v130.0.0`**. Phase XXIV (post-gate hardening). No new gate tag. Final
> core cluster of the hardening sweep (127 → 128 → 129 → **130**); 131–132
> (delivery-table unify, admin audit API) follow.

## What shipped

- **observability env-parsing, now tested** via pure extraction: `is_truthy`,
  `resolve_metrics_endpoint`, `parse_metrics_interval` (metrics.rs) and
  `parse_log_format` (lib.rs). The `*_from_env` wrappers feed `std::env::var(...)`
  into them; 6 unit tests cover truthy parsing, interval defaults, endpoint
  precedence (dedicated > gated-shared, blank = unset), and log-format parsing.
- **maidan-mcp `prompts.rs`** (previously untested) gains a catalog-integrity
  test (non-empty name + description, well-formed arguments, `thread_workflow`
  present).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| server e2e | DB-backed MCP tool-handler coverage | Tool handlers need a store; they're integration-tested via the server, not unit tests. |
| — | A coverage *target* | The 40% floor is enforced; this cluster fills named zero-coverage gaps, not a number. |

## Surprises

- **The race-free path was also the cleaner path.** Wanting to avoid
  `env::set_var` in a parallel test binary forced extracting pure parsers — which
  is better code anyway (the side-effecting `from_env` shrinks to a one-liner
  over a tested pure core). The constraint improved the design.

## Decisions

- **Pure functions over env-mutating tests.** Deterministic, race-free, and they
  document the parsing rules (precedence, defaults, blank-handling) as
  executable spec.
- **Target zero-coverage gaps, not a percentage.** The floor is already enforced;
  the value is covering specific untested logic the scan named.

## Capability table extension

| Capability | Where |
|------------|-------|
| Tested observability env-parsing (pure parsers) | `crates/maidan-observability/src/{metrics,lib}.rs` |
| MCP prompts catalog-integrity test | `crates/maidan-mcp/src/prompts.rs` |

## Risks identified + still open

- **DB-backed MCP paths remain integration-only.** Acceptable — unit-testing tool
  handlers would mean mocking the store; the server e2e suite exercises them
  end-to-end.

## Forward look

Core hardening sweep complete (127–130). Remaining authorized work: **131**
unify the webhook + automation delivery tables, **132** expose the global
cross-workspace admin audit query API.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
