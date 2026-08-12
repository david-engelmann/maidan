# Cluster 200.0 — filtered-ANN search (RBAC deny in the query)

**Theme:** Arc D (performance & scale), part 3 — stop fetching + ranking search
hits the caller can't see. Push the private-channel RBAC deny into the query so a
full page of *accessible* results comes back.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v200.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `deny_channels: Vec<ChannelId>` on `SearchFilters` (RBAC pre-filter, not a user facet) | `maidan-search/src/filters.rs` |
| Query-level exclusion — SQLite `NOT IN (…)`, Postgres `<> ALL($n)` — on lexical + semantic (both backends) | `maidan-search/src/{sqlite,postgres}.rs` |
| `private_channel_deny_set(store, auth, workspace)` — private non-DM channels the caller isn't in | `maidan-auth/src/access.rs` |
| Wire into REST `GET …/search` + MCP `search_messages` | `routes/search.rs`, `tools/search.rs` |

## Why

Message search ran top-K then dropped the hits the caller couldn't access (a
per-thread post-filter, Cluster 162/180). Two problems: it ranked rows that were
about to be thrown away, and — the real wart — it **under-filled the requested
`limit`**. Ask for 10, and if 6 of the top-10 are in a private channel you're not
in, you get 4. The page shrinks silently based on data you can't see.

## The change

Push the private-channel deny into the query so those rows never come back:

- `private_channel_deny_set` computes the private, non-DM channels the caller
  isn't a member of (mirrors the `subscribe_grants` deny logic). `bypass` →
  empty.
- Both backends add a deny clause. SQLite has no array binding, so a dynamic
  `NOT IN (?, …)` with each id bound (`deny_channels_clause_sqlite`), applied to
  the lexical query and both semantic paths (vec + Rust-rank fallback). Postgres
  uses `t.channel_id <> ALL($n)` with a single array bind — an empty array is
  vacuously true, so **no dynamic SQL** and the clause is always present.
- The thread-level post-filter **stays** as the authoritative check. RBAC here is
  thread-level and DM-participant-aware (Cluster 180); DM threads all live in the
  shared `__dm__` channel, so a channel-level pre-filter can't gate them — they're
  intentionally excluded from the deny-set and handled by the post-filter. The
  pre-filter is an optimization, never the sole check.

## Exit criteria

- A non-member's search excludes private-channel hits *in the query* and returns
  a full page of accessible results; DMs still correct — **met**.
- `v200.0.0` tagged.

## Verification & limits

- `assert_deny_channels_filter` in the both-backend common suite: denying a
  channel drops exactly its hits; denying all → empty.
- `search_filtered_ann_e2e` (value proof, auth enabled): 5 private + 3 public
  "widget" matches; a non-member's `limit=3` search returns **exactly 3 public**
  hits (limit honored, would have been a short page under post-filter-only) and
  **no** private-channel content; a member still sees the private hits.
- Existing lexical/semantic/RBAC suites unchanged (empty deny = no behavior
  change).
- Limits (Open Work): the deny-set is `list_channels` + a per-channel
  `channel_is_member` — a single "my private channels" query would be cheaper;
  full DM-at-query-level (removing the post-filter) is deferred (DM participation
  in SQL is complex). The pre-filter duplicates channel-RBAC in SQL, so the
  post-filter is kept as the single source of truth.

## References

- [[Retros/Cluster 200.0]]; `maidan-search/src/filters.rs`,
  `maidan-auth/src/access.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (Arc D). Builds on the RBAC arc
  ([[Retros/Cluster 180.0]]) + the 198 harness.
