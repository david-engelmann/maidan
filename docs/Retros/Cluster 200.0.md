# Cluster 200.0 retro — search stops paying for hits you can't see

> Tag **`v200.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc D (performance & scale), part 3.

## What shipped

- Search pushes the caller's private-channel RBAC deny into the query (SQLite
  `NOT IN`, Postgres `<> ALL`), across lexical + semantic on both backends, so a
  full page of *accessible* hits is returned instead of a post-filtered short
  page — while the thread-level post-filter stays authoritative (DM-aware).

## Surprises / decisions

- **The real bug was a shrinking page, not just wasted work.** I opened this as a
  perf cluster ("don't rank rows you'll discard"), but the sharper problem is
  correctness-of-UX: the post-filter silently under-fills `limit`. Ask for 10,
  get 4, with no signal why. The pre-filter fixes that — the query returns
  `limit` accessible rows — which is the assertion the value test makes.
- **Kept the post-filter as the source of truth.** The tempting move is "push all
  RBAC into SQL, delete the post-filter." I didn't, for two reasons. First, RBAC
  here is *thread-level* and DM-participant-aware; DM threads share the `__dm__`
  channel, so a channel-level query filter can't gate them without replicating DM
  membership resolution in SQL across both backends. Second, having RBAC in two
  places is a real risk — if I deleted the post-filter and the SQL filter had a
  gap, that's a leak. So the SQL deny is an *optimization* layered under the
  authoritative post-filter, and DMs are explicitly out of the pre-filter. The
  test seeds a member + non-member to prove both paths.
- **Postgres was cleaner than SQLite here.** `t.channel_id <> ALL($n)` with a
  single array bind handles the empty case vacuously (`<> ALL('{}')` is true), so
  no dynamic SQL — the clause is always present. SQLite has no array binding, so
  it needs dynamic `NOT IN (?, …)` placeholders bound in a loop, and only when
  the set is non-empty. Two backends, two idioms for the same predicate.
- **`deny_channels` is a filter field but not a *facet*.** It lives on
  `SearchFilters` (that's where the query filters are), but it's server-computed
  RBAC, not user-settable, so `is_empty()` (which gates user-facet behavior)
  deliberately ignores it and the query applies it unconditionally.

## Decisions

- **A `sqlx::query(&format!(…))` gotcha.** Splitting the builder into
  `let mut q = sqlx::query(&format!(…)); … q = q.bind(…)` drops the `format!`
  temporary while `q` borrows it — bind the string to `let sql = format!(…)`
  first. Cost one compile round.
- **Both surfaces.** Wired the deny into REST and the MCP `search_messages` tool
  (both already post-filtered), so the limit-honoring + less-waste win is uniform.

## Capability table extension

| Change | Where |
|--------|-------|
| Query-level private-channel deny for search (lexical + semantic, both backends) | `maidan-search` + `maidan-auth` + `routes/search.rs` + `tools/search.rs` |

## Risks identified + still open

- **RBAC duplicated in SQL.** Mitigated by keeping the post-filter authoritative
  and excluding DMs from the pre-filter — but a future edit to channel-RBAC must
  remember the SQL deny exists. Open (Open Work): the deny-set is N membership
  queries (a single "my private channels" query is cheaper); full
  DM-at-query-level (removing the post-filter) deferred.

## Forward look

Arc D continues: workspace-sharded fan-out + shared reconcile, batched
`pg_notify`, read-replica routing. Each baselined with `loadgen`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Builds on the channel
RBAC arc ([[Retros/Cluster 180.0]]) and the [[Retros/Cluster 198.0]] harness.
