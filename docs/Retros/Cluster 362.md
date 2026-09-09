# Cluster 362 retro — the WIP limit (Wave 1 #13, G11)

Wave 1 #13 bundles wait-edges + `on_timeout` (G2), an escalation enum (G4), and
fair dispatch + Unclaimable + hard WIP (G3/G11). The research is emphatic that the
**WIP limit** is the standout, self-contained win — "one live turn per occupancy",
"409 agent_busy", "WIP must not count queued-never-started **ghosts**". This
cluster ships it end-to-end; the wait-edge/escalation and Unclaimable/fair-dispatch
sub-items are deferred (see below).

An agent can no longer grab unbounded concurrent work: a per-workspace cap bounds
the **live** claims any one member holds.

## What shipped

- **362.1 (#684) — the store foundation (zero-blast).** `maidan_wip_limits`
  (pg 0066 / sqlite 0065): a per-workspace row = the max concurrent live claims per
  member; **no row = unlimited**, **`0` = frozen**. `WorkspaceStore::set_wip_limit`
  (`Some` upsert / `None` clear) + `get_wip_limit`; `AssignmentStore
  ::count_live_claims(member)` — the exact **complement of the `claim_next`
  claimability predicate** (assigned, non-terminal, non-tombstoned, lease not
  expired). New `wip.rs` in both backends; methods on the existing sub-traits.
- **362.2 (#686) — REST enforcement + admin/visibility.** `claim_next` returns
  null at the cap; explicit `claim` returns **409** (a re-claim of a held thread is
  exempt); `routes::at_wip_limit` helper. `PUT`/`GET /workspaces/:wid/wip-limit` +
  `GET /members/:id/wip`.
- **362.3 (#687) — MCP parity.** The same enforcement on the MCP `claim_thread`
  (InvalidParams) / `claim_next_thread` (null), plus `set_wip_limit` /
  `get_wip_limit` / `get_member_wip` tools.

## Decisions

- **Count LIVE claims, never ghosts.** The count is the complement of the
  claim_next predicate: an expired lease is *claimable*, so it is not live work —
  it doesn't count. A durable (no-lease) assignment counts; a terminal thread does
  not, even while still assigned. This is the "WIP must not count
  queued-never-started ghosts" rule made precise.
- **Per-workspace limit, applied per-member.** "In this workspace, each member may
  hold ≤ N live claims." A per-workspace *stored* policy (a table) rather than an
  env global avoids threading config into both the REST `AppState` and the
  `McpServer` — the store enforces from one source of truth, and it's
  multi-tenant-friendly + runtime-settable. `0` freezes claiming (a kill-switch
  lite); no row = unlimited (opt-in, zero behaviour change until configured).
- **Boundary-enforced, not in the claim SQL.** The `claim_next` query is the
  hairiest SQL in the store (readiness + skill + gate clauses). Enforcing WIP there
  would risk that delicate hot path; instead the check is a small pre-dispatch
  helper at the REST/MCP boundary reading `count_live_claims` + `get_wip_limit`.
  The trade-off is a **soft** limit: a member self-racing two concurrent claims
  could momentarily reach limit+1 (the reads aren't in the claim tx). Different
  members never race (WIP is per-member); the occupancy view shows the truth; and
  a rare 1-over for a self-double-dispatching agent is inconsequential. Documented.
- **Explicit claim 409, `claim_next` null.** An explicit `claim` of a specific
  thread past the cap is a loud refusal (409 / InvalidParams) — the caller asked
  for *that* thread. `claim_next` ("give me anything") returns null — the same
  shape as an empty queue; a capped agent's poll simply finds no work.
- **A re-claim is exempt.** Claiming a thread the member already holds is not a new
  slot, so it never 409s (a heartbeat-style re-claim is safe).
- **Enforcement reads hit the primary, not the read replica.** A lagged limit or
  count would mis-enforce; the small extra primary read is worth the correctness.

## Surprises

- Smooth. The `wip.rs`-in-both-backends module kept `backend_parity` happy with no
  registration (unlike a single-backend module). The store's sub-trait split meant
  no new sub-trait — the methods slotted onto `WorkspaceStore`/`AssignmentStore`.

## Test evidence

- Store: `wip_limits` both backends (set/get/clear + freeze; count reflects
  durable assignments, drops on unassign, excludes terminal); backend/dialect
  parity.
- REST: `wip_limit_e2e` (cap=1 → claim OK / second claim 409 / claim_next null /
  clear → OK); openapi bijection + capability matrix.
- MCP: `wip_limit_tools_enforce_the_cap`; both contract-sync tests.

## Forward look

The hard WIP limit (G11 / the G3 WIP half) is shipped end-to-end. **Deferred —
the rest of Wave 1 #13, each a distinct build (logged in Open Work):**
- **G2 wait-edges + `on_timeout` / G4 escalation enum** — a dependency/timer with
  a deadline that fires an escalation *policy* on timeout. **Critical constraint
  from the research:** `on_timeout` is **no-decision / park**, never an invented
  human refusal (the "TimedOut ≠ Decline" rule) — steal the Restate/Temporal
  promise/timer *shape*, not the engine.
- **G3 Unclaimable** — a thread `claim_next` skips (skill-miss is already implicit
  via Cluster 231; extend to an explicit park / start-failure `rejected`).
- **G3 fair dispatch** — anti-starvation ordering in `claim_next` (today: oldest
  first).

Next-ranked after those is **Wave 1 #14** (N1 web-push / T6 legal-hold / H15 OTel
gate / SCIM-as-OIDC-P3).

## Acknowledgements

Built as a three-PR run (#684 → #686 → #687) on the Cluster-190/192/351 claim
lifecycle + the Cluster-349 store sub-trait split — each rebased onto `main` as its
parent merged.
