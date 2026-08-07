# Cluster 175.0 — token: MCP `search_messages` snippet_only parity

**Theme:** Arc 4 (token round 3), part 1 — bring the REST `snippet_only`
token-saver to the MCP search tool.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v175.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `snippet_only` arg on the MCP `search_messages` tool → drops full `body`, keeps the snippet | `maidan-mcp/src/tools/search.rs`, `catalog.rs` |

## Why

REST search has had `snippet_only=true` since Cluster 152 (drop the full message
`body` from each hit; semantic hits get a UTF-8-safe truncated body prefix as the
snippet). The MCP `search_messages` tool — the one agents actually call — still
returned full bodies, the largest token cost in a result set. This is the
parity: the same `SearchHit::into_snippet_only` applied after the Cluster 162
channel-access filter.

## Key decisions

- **Reuse `SearchHit::into_snippet_only`** (the REST path's helper) — identical
  semantics across surfaces, one source of truth (`SNIPPET_FALLBACK_BYTES=240`).
- **Default `false`** — opt-in, no behavior change for existing callers.
- No new capability / tool / contract (just an argument + a catalog property).

## Exit criteria

- MCP `search_messages` with `snippet_only:true` returns hits with blanked
  bodies + retained snippets; suites green — **met**.
- `v175.0.0` tagged.

## Verification & limits

- `mcp_search_messages_snippet_only_drops_bodies` (search_e2e): `tools/call
  search_messages` with `snippet_only:true` → every hit's `body` is `""` and its
  snippet is non-empty. The existing full-body MCP search test still passes
  (opt-in). `into_snippet_only` itself is unit-tested in `hit.rs` (Cluster 152).

## References

- [[Retros/Cluster 175.0]]; `maidan-mcp/src/tools/search.rs`,
  `maidan-search/src/hit.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (token round 3).
