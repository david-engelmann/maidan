# Cluster 176.0 — token: capability-filtered `tools/list`

**Theme:** Arc 4 (token round 3), part 2 — return only the tools a caller can
actually invoke, shrinking `tools/list`.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v176.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `tools/list` filters the catalog to tools whose required capability the caller holds | `maidan-mcp/src/tools/mod.rs` (`catalog_for`), `server.rs` |

## Why

`tools/list` returned the **entire** catalog regardless of the caller's token
capabilities. A capability-scoped agent (say, a read-only or search-only token)
paid tokens for ~40 tool schemas it would only ever get `403`s from, and had to
reason about tools it can't use. Now the list is filtered to the caller's
capabilities via a new `tools::catalog_for(auth)`, which reuses the existing
`required_capability(name)` mapping.

## Key decisions

- **Filter in the `tools/list` arm, not in `catalog()`.** The unfiltered
  `catalog()` is unchanged, so the catalog↔contract tests and full-capability
  callers are unaffected; only the *response* is scoped.
- **Bypass sees everything.** Auth-disabled callers (dev/tests) get the full
  list, as before.
- **Reuse `required_capability` + `has_capability`.** One source of truth for
  the tool→capability mapping; no per-tool metadata duplication.

## Non-goals

- Trimming the verbose catalog *descriptions* (a separate token lever) — deferred
  within arc 4; the bigger win is not sending whole unusable tool schemas.

## Exit criteria

- A capability-scoped token's `tools/list` contains only tools it can call;
  bypass/full-cap callers unchanged; suites green — **met**.
- `v176.0.0` tagged.

## Verification & limits

- `catalog_filter_tests` (maidan-mcp unit): bypass sees the whole catalog; a
  `workspace:read`-only token sees only read tools (and every surfaced tool
  really requires `workspace:read`); a `search:query`-only token sees exactly
  `search_messages`. The dispatch wiring (`catalog_for(auth)`) is a
  compile-checked one-liner over the same `auth` the arm already receives.
- Limit: filtering is by the tool's single required capability; tools aren't
  hidden for other reasons (e.g. a tool needing a streamable session still
  appears — capability, not session-state, is the filter).

## References

- [[Retros/Cluster 176.0]]; `maidan-mcp/src/tools/mod.rs`,
  `maidan-mcp/src/server.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (token round 3).
