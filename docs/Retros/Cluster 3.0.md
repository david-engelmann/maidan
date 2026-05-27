# Cluster 3.0 retro — Search & subscriber depth

> Closing wave for Cluster 3.0 · target tag `v3.0.0`.

Cluster 3.0 closed the top deferred search/reliability items: semantic facets,
automatic replay on subscriber lag, and a coverage floor in CI.

## What shipped

- **PR #146** — semantic search facets on Postgres (`author` / `channel` / `kind`) across HTTP + MCP.
- **PR #147** — WS/MCP auto-replay from `maidan_events` on bus lag when `workspace_id` filter is set.
- **PR #148** — CI coverage line gate via `cargo llvm-cov --fail-under-lines` with documented baseline policy.

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| Post-3.0  | SQLite semantic search (`sqlite-vec`)             | Extension maturity / operational risk.   |
| Post-3.0  | Full resumable WS tokens beyond `after_id`        | Larger protocol design than this cluster. |
| Post-3.0  | Codecov upload and per-crate thresholds           | Kept one global floor first.             |

## Surprises

- `cargo llvm-cov --summary-only` was slow/unreliable in this environment; using CI artifacts gave a stable baseline.
- Hash-embedding tests can still return cross-matches; facet e2e assertions should verify ID inclusion/exclusion, not only empty sets.

## Decisions

- **Semantic facet parity** — Postgres semantic mode now honors lexical facets.
- **Auto-replay default** — replay automatically when workspace scope is known; retain `replay_hint` fallback otherwise.
- **Coverage floor policy** — start at 9.0% from a measured 9.8% baseline and raise only via dedicated CI/docs PRs.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Semantic facets on Postgres (`mode=semantic` + facets) | `v3.0.0`           |
| WS/MCP auto-replay from `maidan_events` on lag         | `v3.0.0`           |
| CI line-coverage fail-under gate                        | `v3.0.0`           |

## Risks identified + mitigated

- **Semantic facet divergence** — single `SearchFilters` path for lexical + semantic call sites.
- **Lag gaps requiring manual replay** — auto-replay wired in shared transport helper.
- **Coverage regressions** — CI now fails below documented floor.

## Risks identified + still open

- **Global coverage remains low** (~10%); floor prevents regression but does not by itself improve coverage depth.
- **Replay truncation** at `REPLAY_LIMIT=500` still requires clients to continue from `after_id` if gaps exceed one window.

## Forward look

Next cluster planning is open; likely candidates are coverage uplift + search quality follow-ups from [[Open Work]].

## Acknowledgements

Solo cluster. Three implementation PRs plus this retro.
