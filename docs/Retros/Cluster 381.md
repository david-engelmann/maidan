# Cluster 381 retro — Wave 2 #24 facet half: `result_kind` is a namespaced-string list

Wave 2 #24 asked for two things: the next `claim_next` claimer must see
in-channel **accepted/closed decisions**, and `result_kind` must be a search
facet. Cluster 382 shipped the pack half. This cluster is **only the facet
half**.

The facet is a **namespaced string**, never a closed enum. A live pi result
publishes `result_kind = "pi.review.result/1"` inside `schema =
"pi.waiter.result/1"`. The old Open Work guess (`decision|plan|merge_authorized`)
is not the wire vocabulary and is not a filter value. The ADR convention
`"kind": "decision"` is a different field.

The surface is a workspace-scoped **list**, not `GET /workspaces/:id/search`.
Match is exact. `pi.review.result` and `pi.review.result/10` do not hit
`pi.review.result/1`.

Four impl PRs (381.1–381.4) + this retro. 381.4 documented the facet; it is
**not** this retro. Clusters 380 and 382 are already closed. This retro does
not start Wave 2 #25 / Cluster 383.

## What shipped

- **381.1 (#798) — store.** `result_kind_from_payload` reads the string alone
  (trim; missing / empty / whitespace / non-string → `None`). It does **not**
  require `schema = "pi.waiter.result/1"` — a future producer kind is
  first-class without an envelope change. Indexed on `maidan_thread_results`
  (pg 0087 / sqlite 0086); `set_thread_result` writes or clears the column.
  `Store::list_thread_results(workspace, result_kind, limit)` on both
  backends: exact match when `Some`, every non-tombstoned result when `None`.
  Cluster 382's `list_channel_closed_results` is untouched.
- **381.2 (#799) — REST.** `GET /workspaces/:id/results?result_kind=&limit=`
  (`workspace:read`) over that list. Absent `result_kind` = every accessible
  non-tombstoned result. Private-channel rows dropped via `can_access_thread`.
  Full new-route preflight (OpenAPI stub + `paths(...)` +
  `http-capability-map.json` GET entry). `limit` default 50, clamp 1–500.
- **381.3 (#801) — MCP twin.** `list_thread_results` (`workspace:read`):
  workspace from `auth.workspace_id` (no `workspace_id` argument), same
  optional `result_kind` / `limit`, same thread-RBAC filter. Standard 5-place
  wiring + both sorted `contracts/mcp-*.json`. The pre-dispatch gate cannot
  cover an aggregate read, so the filter is in-handler (the
  `list_assigned_threads` shape).
- **381.4 (#802) — Integration + Result Delivery.** Discoverability is the
  namespaced string. Decision-records note that `"kind": "decision"` is not
  `?result_kind=decision`. **Not a retro** — Open Work stayed open until this
  close.
- **381.5 — this retro + the doc-close.** Strike row #24 completely (382 pack
  + 381 facet).

## Decisions

- **`result_kind` is a namespaced string, not a closed enum.** An enum would
  need editing every time a waiter product ships a new kind. The store
  indexes whatever non-empty string the payload carries.
- **A new workspace-scoped list, not a filter on `list_channel_closed_results`.**
  382.1 is the claimer-pack query (same-channel, terminal, newest
  `produced_at`). Changing its signature would have broken in-flight 382.2 /
  382.3. Discoverability is a different product: every result in the
  workspace, optionally narrowed by kind.
- **The facet does not require the waiter schema.** `result_kind_from_payload`
  reads the string alone. A producer that is not pi is still listable.
- **List, not message-FTS.** `SearchFilters` is `posted_at` / body / channel
  deny. Thread results are not messages. Wiring the facet through
  `GET …/search` would have mixed two indexes and two RBAC stories.
- **Exact match.** Prefix and sibling versions (`/1` vs `/10`) must not collide.
  Empty / whitespace `result_kind` on the store list is the same as no filter.
- **The ADR `"kind": "decision"` convention is not this facet.** That JSON
  lives in the payload as `kind`. A payload that only has `"kind": "decision"`
  will not match `?result_kind=decision`. Documented on both Integration.md
  and Result Delivery.md so the next reader does not "helpfully" alias them.

## Surprises

- **382.1 landed during the 381.1 survey** and explicitly declined to facet
  (`"that's Cluster 381"`). A shared signature would have coupled two
  in-flight clusters. Separate method, zero blast on the pack.
- **A stacked PR targeting a deleted base is closed, not retargeted.** Same
  lesson as 379.4 / #790, 382.2 / #795, 380.2. 381.3 (#801) and 381.4 (#802)
  rebased onto `main` after the 381.2 squash and dropped the parent commits.
  381.4 also dropped the still-unmerged 381.3 MCP commit so the docs PR would
  not go `CONFLICTING` when #801 landed.
- **A GET list still needs the OpenAPI `paths(...)` entry *and* a
  capability-map row with the same `{id}` template**, or `openapi_e2e`
  bijection fails (Cluster 187). ThreadResult was already registered.
- **Wave 3 #30 (the EventKind JSON-Schema pack) does not exist yet.** 381.4
  kept Result Delivery in step and did not invent that pack. Registering
  `pi.waiter.result/1` there stays Wave 3 work.

## Test evidence

- Types: `result_kind_from_payload` — `pi.review.result/1` extracted; empty /
  whitespace / non-string / `"kind": "decision"` → `None`; a free-form
  `acme.plan.result/2` is equally first-class. Fixture lock still carries
  the waiter `result_kind`.
- Store, both backends: `result_kind` — exact match; prefix and `/10` sibling
  miss; the old enum word `decision` misses; `None` lists every
  non-tombstoned result including unkinded rows; another workspace does not
  leak; empty/whitespace kind = unfiltered; newest `produced_at` first;
  `limit` honored; re-set moves the row from the old kind to the new.
  `dialect_parity` + `backend_parity` + `concurrent_migrations` green.
- Server: `result_kind_rest_e2e` (auth-enabled, minted `workspace:read`) —
  unauthenticated 401; unfiltered omits a private-channel row the caller
  cannot access; `?result_kind=pi.review.result/1` is exact; `?result_kind=decision`
  is empty. OpenAPI bijection + `http_capability_matrix_e2e`.
- MCP: `list_thread_results_filters_by_namespaced_kind` — unfiltered lists
  accessible rows; namespaced filter hits; `decision` misses; private
  withheld. Catalog + both contracts sorted.
- Docs: `mdbook build` with the linkcheck renderer (`docs/Result Delivery.md`
  and `docs/Integration.md` are published). 381.4 already staged the
  discoverability copy.

## Forward look

**Cluster 381 is complete. Row #24 is closed** (382 pack + 381 facet). A
caller can list workspace thread results by the namespaced `result_kind`
string a producer actually publishes.

**The result-delivery arc (377–381) is COMPLETE:** durable projector egress
(377), allowlist + sender upgrade + body projection (378), the delivery
primitive (379), inline per-finding PR comments (380), and this facet (381).
Clusters 380 and 382 stay closed.

Deferred (follow-ups): pushing the deny-set / private-channel filter into
the list query (REST and MCP still post-filter via `can_access_thread`, so
`limit` is applied before RBAC — a private-heavy page can under-fill);
faceting 382's in-channel closed list by kind (declined — different product);
Wave 3 #30 registering `pi.waiter.result/1` in the schema pack (not this
cluster).

**Not this cluster:** Wave 2 #25 (Soundcheck) stays the next unstruck
*product* row. This retro does not start it and does not invent Cluster 383.
P1.1d (`transition_thread` MCP) was not taken.

## Acknowledgements

Four impl PRs (#798 store → #799 REST → #801 MCP → #802 Integration + Result
Delivery) + this retro. **381.1 and 381.2 were on `main`** before 381.3
opened. 381.3 (#801) and 381.4 (#802) rebased onto `main` after the 381.2
squash and dropped the parent commits so the siblings would not go
`CONFLICTING` / `base_ref_deleted`.
