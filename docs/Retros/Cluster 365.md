# Cluster 365 retro — fair dispatch (Wave 1 #13, G3; #13 complete)

The last open sub-item of Wave 1 #13. `claim_next` was strict FIFO
(`created_at ASC`) — the oldest eligible thread always dispatched next. That is
technically starvation-free but expresses **no urgency**: you cannot say "do this
first". This cluster adds a **dispatch priority** — and, crucially, an **aging**
term so priority jumps the queue *without* re-introducing starvation.

## The idea in one line

`effective_rank = base_priority + floor(age_seconds / 3600)`, ordered DESC with
`(created_at, id)` ASC as the tiebreak. A high-priority task jumps ahead; a
long-waiting normal task ages one rank per hour until it overtakes newer
higher-priority work — so nothing starves. **Priority alone would starve
low-priority tasks; the aging is the whole reason this is called *fair* dispatch.**

## What shipped

- **365.1 (#708) — the store foundation (zero-blast).** `maidan_thread_priorities`
  (pg 0069 / sqlite 0068): one row per thread, `{priority, set_by, set_at}`;
  absence = the default 0. `ThreadPriority` + `AssignmentStore::set_thread_priority`
  (upsert) / `get_thread_priority`. A side table (not a `maidan_threads` column) to
  keep the ~46-site `row_to_thread` ripple off the hot path. pg column `BIGINT` for
  a clean i64.
- **365.2 (#709) — the aged-rank ordering (the actual dispatch change).** Both
  `claim_next` variants × both backends LEFT JOIN the priorities table and ORDER BY
  the effective rank. Postgres `FLOOR(EXTRACT(EPOCH FROM (NOW()-created_at))/3600)`
  (with `FOR UPDATE OF c` so the JOIN doesn't widen the lock); SQLite
  `CAST((strftime('%s','now')-strftime('%s',created_at))/3600 AS INTEGER)`.
- **365.3 (#710) — REST.** `PUT`/`GET /threads/:id/priority`.
- **365.4 (#711) — MCP.** `set_priority` / `get_priority`.

## Decisions

- **Aging, not bare priority.** Bare priority ordering starves the low end under a
  steady stream of urgent work. The one-boost-per-hour aging is what earns the
  "anti-starvation" name the backlog item asks for; it is also the classic OS
  scheduler answer (priority aging), so it steals a known-good shape rather than
  inventing one.
- **The ordering change lives in the SQL, not a new store method.** Priority is a
  pure `ORDER BY` term — no new bind param on the claim path, no new capability, and
  the REST claim-next route + MCP `claim_next_thread` tool become fair-dispatch for
  free. The explicit by-id `claim`/`claim_with_event` are untouched: they claim one
  *named* thread, not "the next best", so priority is irrelevant there.
- **A side table, default 0 by absence.** Consistent with 362/363/364 (wip /
  unclaimable / waits) and dodges the thread-column ripple. `get` returns `None`
  (REST 404, MCP null) when unset — the caller reads that as "the default 0".
- **A fixed 1-hour aging window.** Inlined as the `3600` divisor in the SQL
  (documented; the test mirrors it as `AGING_WINDOW_SECS`). Per-workspace tunability
  is a possible later refinement, not needed for a first cut.

## Surprises

- **SQLite `strftime` had to be verified before I trusted it.** If
  `strftime('%s', created_at)` returned NULL on our RFC3339 timestamps, the whole
  aging term would silently collapse to NULL and the ordering would fall back to
  FIFO — aging quietly disabled, no error. A quick `sqlite3` check confirmed it
  parses `2026-…T…+00:00` (with fractional seconds and the offset) to a real epoch;
  the both-backend aging test is the standing guard.
- **The pg LEFT JOIN forced `FOR UPDATE OF c`.** A bare `FOR UPDATE SKIP LOCKED`
  over a join tries to lock rows of *both* tables; naming the candidate (`OF c`)
  keeps the lock on the thread row alone, preserving the concurrent-claimer
  semantics.
- **The `set_by` FK, a fourth time.** Both `set_priority` surfaces persist
  `auth.member_id`, so the nil-member `for_tests` bypass FK-fails; the REST e2e runs
  auth-enabled with a minted token and the MCP test uses a real `from_session`
  member (same as 363.3/364.4/364.5).

## Test evidence

- Store: `thread_priorities` (set/get/default-absent, both backends);
  `thread_priority_dispatch` (both backends) — priority jumps FIFO (t3>t2>t1), equal
  priority keeps oldest-first, and a **3h-aged priority-0 task overtakes a new
  priority-2 one** (the anti-starvation property, via a per-backend `created_at`
  backdate). Re-ran deps/skills/unclaimable/gate/assignment/wip + dialect/backend
  parity — all green (claim_next is a shared hot path).
- Server: `priority_rest_e2e`; openapi bijection + capability matrix.
- MCP: `priority_tools_set_and_get`; both contract-sync tests.

## Forward look

**Wave 1 #13 is now COMPLETE** — G11 WIP (362), G3 Unclaimable (363), G2/G4
wait-edges (364), and G3 fair dispatch (365). **Deferred (logged):** per-workspace
aging-window tunability; surfacing priority in `QueueDepth`/occupancy (it changes
ordering, not the ready/blocked partition, so it was left out). **Next: Wave 1 #14**
— N1 web-push / T6 legal-hold / H15 OTel gate / SCIM-as-OIDC-P3 (four bullets, not
one cluster).

## Acknowledgements

Built as a five-PR run (#708 → #709 → #710 → #711 → this retro) on the
Cluster-362/363/364 side-table + claim_next-clause patterns.
