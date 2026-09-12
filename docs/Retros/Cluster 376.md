# Cluster 376 retro — Wave 2 #23: a spawn budget (G6 + G-dev-3 + W3)

Wave 2 #23 puts a ceiling on agent fan-out. A workspace declares how far an
agent family may spread — how many direct children a parent claim may hold, how
deep the nesting may go, how many tool calls one thread may record — and the room
refuses the spawn past the cap instead of watching a runaway recruit helpers.
Brooks/Amdahl/Two-Pizza: coordination cost grows as n(n−1)/2, so admitting one
more agent onto a late sequential claim makes it later. A claim's **external**
fan-out is capped too: at most one GitHub issue/PR link per claim.

**A budget, not a scheduler.** It does not decide who works, re-plan, or kill a
running agent (that is the Cluster-372 freeze). It stands between a claim and its
next helper.

## What shipped

- **376.1 (#765) — the store foundation.** `maidan_spawn_budgets` (pg 0080 /
  sqlite 0079; `workspace_id` PK, three nullable limit columns) + `SpawnBudget` +
  `SpawnBudgetStore` (both backends): `set` (whole-row upsert; all-`None` deletes
  the row) / `get`, plus the three counts the gate reads —
  `count_active_children` (non-tombstoned direct children), `thread_depth` (an
  ancestor recursive-CTE, root = 1), `count_thread_tool_uses` (`tool_use` blocks
  across a thread's messages' Cluster-173 `content`). Zero-blast-radius.
- **376.2 (#768) — children + depth enforcement.** `enforce_spawn_budget` right
  after `validate_parent` in **both** create paths (`create` +
  `create_with_event`, both backends), so every thread-create path (REST,
  recipes, the scheduler) inherits it. Root threads and no-budget workspaces are
  unrestricted.
- **376.3 (#770) — max-tools enforcement.** `enforce_tool_budget` in both post
  paths, keyed on "this post carries tool-use blocks" so an ordinary message
  never pays for a budget lookup. A *cumulative thread* cap, not a per-post one.
- **376.4 (#771) — the config surface.** REST `PUT`/`GET
  /workspaces/:id/spawn-budget` (`workspace:write`/`workspace:read`) + MCP
  `set_spawn_budget`/`get_spawn_budget`. `PUT` is a full replace (an omitted axis
  is unlimited, `{}` clears the budget, `0` freezes an axis); `GET` is total (all
  axes `null` when unset, no 404).
- **376.5 (#772) — the GitHub-link cap.** The reverse index on
  `maidan_github_issue_links(thread_id)` becomes **UNIQUE** (pg 0081 / sqlite
  0080), completing the bijection with the existing `(repo, issue_number)` key:
  one thread per issue *and* one issue per thread. The store maps the violation
  to a `Conflict` via the established `is_unique_violation` pattern; no route or
  tool changed.
- **376.6 (#774) — `ThreadSpawnDenied`.** A refusal is now a room event
  (`{workspace, channel, thread, member, axis, limit, observed}`,
  non-federatable) published from the REST thread-create route, both REST
  message-post branches, and the MCP post tool. The gate's error became a typed
  `StoreError::SpawnRejected(SpawnDenial)` to carry the payload.
- **376.7 — this retro + the doc-close.**

## Decisions

- **The gate lives in the store, not the routes.** Both `create`/`create_with_event`
  and both post paths call it, so REST, MCP, recipes and the scheduler are
  covered by one implementation per backend. A route-level gate would have had to
  be re-added at every future spawn site.
- **Each axis is opt-in; absent = unlimited.** An absent row *or* a `NULL` column
  is unlimited on that axis — the Cluster-362 WIP-limit shape. Existing
  deployments are unaffected until an operator sets a cap, and `0` is a
  meaningful value on every axis ("freeze it").
- **`PUT` is a full replace, not a per-axis merge.** The store's `set` was
  already a whole-row upsert, so `PUT` is the honest verb and `{}` genuinely
  means "no caps". A `PATCH`-style merge would have needed a second store method
  and a read-modify-write race; the e2e asserts the replace semantics so nobody
  "fixes" it into a merge later.
- **`GET` is total, not 404-on-unset.** The two precedents in the repo diverge
  (`GET …/result` 404s; `GET …/wip-limit` returns `null`). The budget follows the
  WIP limit: it is the same kind of object, and a client reading "how much
  fan-out is left" should not have to branch on a status code.
- **No maximum on the caps.** The product advice is to set them well below what a
  hosted agent platform allows, and that advice lives in the docs and the MCP
  tool description — not in a hard-coded ceiling nobody asked for.
- **The GitHub cap is a unique index, not a store check.** An earlier draft read
  the thread's links and refused a second one; that is racy on Postgres (two
  linkers both see zero). Expressing the product rule as a schema constraint made
  the check disappear *and* the guarantee stronger — it now holds for every
  insert path, including ones that don't exist yet.
- **The `axis` vocabulary is exactly the three budget columns.** `children` |
  `depth` | `tools`. The GitHub-link cap keeps a plain `Conflict` rather than
  becoming a fourth axis: it is a schema invariant with no configurable limit and
  no meaningful `observed` count, and keeping `axis` 1:1 with `SpawnBudget`'s
  columns means an operator reading a denial knows exactly which knob to turn.
- **The denial event's actor is `Option<MemberId>`.** The store's thread-create
  path has no author (`NewThread` carries none), so the route supplies the
  caller. Under bypass auth there is no real caller, and the audit trail had
  already settled that case: record "unattributed" rather than write the nil
  member id as though someone did it.
- **A refused spawn is not a notification.** `notifiable_kinds` is untouched — an
  operator subscribes to the event; a per-member inbox entry for one's own
  refusal is noise.

## Surprises

- **The `max_tools` gate's cheapness is its shape.** Keying enforcement on "this
  post carries tool-use blocks" (rather than "a budget exists") means the
  overwhelmingly common plain post never touches the budget tables at all.
- **The upsert made error attribution easier, not harder.** Because
  `(repo, issue_number)` is consumed by `ON CONFLICT`, a unique violation
  reaching the caller of `link_github_issue` can only be the new per-thread
  index — so the mapping needs no constraint-name sniffing.
- **The typed error paid for itself immediately.** Converting `Conflict` →
  `SpawnRejected` broke the 376.4 MCP test, which had been matching on a message
  substring; it now asserts `axis == SpawnAxis::Children`. The compiler found the
  one place a string match was standing in for a fact.
- **Bypass auth reports the nil member.** `AppState::for_tests` disables auth, so
  the first attribution e2e "passed" the 409 and failed the actor assertion. The
  denial e2e runs with auth **enabled** and a minted token, because attribution
  is exactly the thing a bypass run cannot prove (the Cluster-228 lesson,
  re-learned from the other direction).

## Test evidence

- Store, both backends: `spawn_budget` (set/get/clear, children + depth over a
  root/child/grandchild tree, tool-use counting over `content` with
  null-content ignored) and `spawn_enforce` (the 3rd child past
  `max_children=2`, the depth-3 grandchild past `max_depth=2`, the 3rd tool call
  past `max_tools=2`, and that clearing re-opens spawning) — the latter now
  asserting the typed denial's `axis`, `limit`, `observed` and scope, plus that
  the client-facing message still names the axis.
- Store, both backends: `github_links` extended — a 2nd distinct issue on a
  linked thread refused; re-linking the same issue idempotent; moving a link onto
  an already-linked claim refused (the `DO UPDATE` path, not just `INSERT`);
  moving to a free thread works and the old thread loses its link.
  `concurrent_migrations` + `dialect_parity` + `backend_parity` green.
- Server: `spawn_budget_rest_e2e` (auth-enabled — round-trip, the set cap is what
  the gate enforces, full-replace, clear, `400` on negative, `403` for a
  read-only token) and `spawn_denied_event_e2e` (auth-enabled — the 409, the bus
  event with the real caller, the durable log row, and no event on an accepted
  spawn). `openapi_e2e` bijection + `http_capability_matrix_e2e` +
  `projector_links_e2e` + `github_ingress/egress_e2e`.
- MCP: `spawn_budget_tools_set_get_and_gate_the_spawn`,
  `a_refused_tool_post_publishes_thread_spawn_denied`, both contract-sync tests.
- Types: the EventKind round-trip + federatable guards and both event-kinds
  contracts cover `thread_spawn_denied`.

## Forward look

**Wave 2 #23 is complete** — the budget is enforced (children, depth, tools),
settable (REST + MCP), capped externally (one GitHub link per claim), and
observable (`ThreadSpawnDenied`).

Deferred (follow-ups): a `SpawnAxis::GithubLink` if the link cap ever becomes
configurable; per-**channel** or per-**member** budgets (workspace-wide today); a
`/ui` panel for the budget (the operator console does not surface the WIP limit
either); a read that answers "how much fan-out is left on *this* parent" without
attempting a spawn (`get_queue_depth` is the nearest thing today); and treating a
denial as a notification for the router if operators ask for it.

**Next: the result-delivery arc, Clusters 377–381** — see the "Result delivery —
the external last mile" section of [[Open Work]] and the pinned contract in
[Result Delivery](../Result%20Delivery.md).

## Acknowledgements

Six impl PRs (#765 store → #768 children+depth → #770 max-tools → #771 config →
#772 GitHub cap → #774 the denial event) + this retro, on the
foundation-then-wire + new-route-preflight + full-EventKind-drill patterns.
