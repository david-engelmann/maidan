# Cluster 372 retro — Wave 2 #20: a freeze-member kill-switch (G17 + B25)

Wave 2 #20 gives an operator a **kill-switch for one member**: freeze a
compromised or runaway agent and it (1) has its active leases dropped, (2) is
refused by `claim_next`, and (3) stays frozen until an explicit unfreeze — the
freeze *is* the gate. **Not G4 PAUSE** (which pauses a thread or workspace); this
stops one member's participation. Plus a catalog of the `MAIDAN_*` kill-switch
flags as operator documentation.

## What shipped

- **372.1 (#747) — the store foundation.** `maidan_member_freezes` (pg 0077 /
  sqlite 0076) + `MemberFreeze` + `MemberFreezeStore`: `freeze_member` records the
  freeze AND releases the member's active claims (drops leases) in **one
  transaction**, returning the freeze + the count released; plus unfreeze /
  is-frozen / get / list, both backends. Zero-blast-radius.
- **372.2 (#748) — claim enforcement.** Both `claim_next` variants gain an `AND
  NOT EXISTS (… maidan_member_freezes f WHERE f.member_id = <claimer>)` clause
  beside the Cluster-363 unclaimable clause — a frozen member matches no
  candidate. Atomic (same statement as the claim UPDATE), so a member frozen
  mid-flight can't slip a claim through.
- **372.3 — REST.** `POST/DELETE/GET /members/:id/freeze` +
  `GET /workspaces/:wid/frozen-members`, gated `token:admin` (an operator tool);
  freeze/unfreeze are audited.
- **372.4 — MCP.** `freeze_member` / `unfreeze_member` / `list_frozen_members` —
  the first `token:admin` MCP tools, so an orchestrator agent can kill-switch a
  misbehaving member.
- **372.5 — the `MAIDAN_*` operator-doc catalog + this retro.**

## Decisions

- **The freeze is the gate.** Rather than force the Cluster-350 approval-gate
  machinery (which is thread-scoped) onto a member-scoped action, the durable
  freeze record itself is the barrier: `claim_next` refuses while it exists, and
  an operator unfreeze is the explicit lift. A `MemberFrozen` *event* (for the
  notification router / UI to react) is a noted follow-up.
- **Freeze drops leases in the same tx as recording the freeze.** So there's no
  window where a member is frozen but still holds work — the release and the
  freeze commit together; the freed threads return to the queue for another agent.
- **Enforce in the claim SQL, not a pre-check guard.** A `is_member_frozen` guard
  before the claim would leave a TOCTOU window (frozen between the guard and the
  claim → one claim slips through, and it wouldn't be auto-dropped). The
  `NOT EXISTS` clause is in the claim's own statement, so it's atomic.
- **`token:admin`, an operator surface.** Freezing is a privileged kill-switch —
  the same bar as legal-hold / SCIM. The MCP tools are the first `token:admin`
  tools, for an orchestrator agent acting as an operator.

## Surprises

- **The sqlite claim binds are positional, and the freeze `?` is 8th / 4th.** pg
  reuses the already-bound `$1` (member) for free; sqlite needed one extra
  `.bind(member_id.0)` per claim site, placed after the skills `?`.
- **A new capability ripples into TWO exhaustive deny-caps matches (again).** The
  http one I'd internalized; the MCP `mcp_deny_caps` panics on an unknown cap too,
  so `token:admin` needed an arm there. This time I ran BOTH matrix tests locally
  before pushing (the 371.3 lesson held).

## Test evidence

- Store: `member_freezes` (freeze drops the lease + records it, idempotent
  re-freeze, unfreeze) + `freeze_claim` (a frozen member is refused by both claim
  variants, unfreeze restores) — both backends.
- Server: `freeze_rest_e2e` (auth-enabled + token:admin — freeze/get/list/deny/
  unfreeze/404) + the bijection / matrix / openapi↔map contracts.
- MCP: `freeze_tools_freeze_unfreeze_and_list` + the catalog / capability-map
  contracts + the mcp capability matrix.

## The `MAIDAN_*` kill-switch catalog

372.5 adds a "Kill switches" section to `docs/Operations.md` cataloguing the
operator levers: the per-member freeze API (this cluster), plus the existing
`MAIDAN_*` flags — `MAIDAN_ALLOW_INSECURE_NO_AUTH` / `AUTH_DISABLED` (fail-closed
auth, 157), `MAIDAN_RATE_LIMIT_MAX` (DoS floor, 183), `MAIDAN_MAX_BODY_BYTES`
(body cap, 183), `MAIDAN_SECRET_EGRESS_ALLOWLIST` (secret egress, 371),
`FEDERATION_DISABLED` (federation ingress), the opt-in workers
(`MAIDAN_*_TICK_SECS`), and the read-replica / retention toggles.

## Forward look

**Wave 2 #20 is complete.** Deferred (follow-ups): a `MemberFrozen` event (for the
notification router / UI); a freeze *expiry* (auto-unfreeze after a window); a
per-workspace freeze reason taxonomy; freezing surfacing on the `/ui`. **Next:
Wave 2 #21** (H11 — attachable labeled memory as room objects, the Letta-block
shape).

## Acknowledgements

Four impl PRs (#747 store → #748 enforcement → 372.3 REST → 372.4 MCP) plus this
retro + operator docs, on the foundation-then-wire + new-route-preflight +
capability-registry patterns.
