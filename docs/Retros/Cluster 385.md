# Cluster 385 retro — Wave 2 #25 remainder: Soundcheck gate pointer + green/amber/red

Wave 2 #25 asked for a Soundcheck pointer on the thread and a
**green/amber/red** land-gate vocabulary so the FSM will not `closed` on
accepted nonsense. Cluster 383 shipped the *composition* half (critical
waiter findings → Cluster-375 `request_changes`). This cluster is the
**remainder**: the room holds `{kind:"soundcheck", status, artifact_sha?,
land}`; Soundcheck owns test execution; Maidan will not treat amber
(flags-then-still-engages) as a land.

Cluster **384** is already claimed by P1.1d (MCP `transition_thread`,
#816/#823). This work is **385**. Do not confuse the two.

Four impl PRs (385.1–385.4) + this retro (#831). Every PR targets `main`
(cumulative) so a parent squash cannot `base_ref_deleted` a child.

## What shipped

- **385.1 (#819) — types + store.** `SoundcheckStatus` (`pass`/`fail`),
  `LandColor` (`green`/`amber`/`red`), `SoundcheckPointer`,
  `SoundcheckStanding`. Table `maidan_thread_soundcheck` (pg 0088 /
  sqlite 0087). `require_soundcheck` arms; `set_soundcheck_pointer`
  upserts from a `soundcheck`-skilled member; `get_soundcheck_standing`
  is total (no row = vacuous green); `clear_soundcheck` disarms.
  Qualifying land = green pass + skill + recorder ≠ owner/assignee.
  Fail is always red. Unskilled writes are `InvalidInput`.
- **385.2 (#821) — FSM close-gate.** `transition_in_tx` (both backends)
  refuses `closed` when a row exists unless the pointer is a qualifying
  green pass. Amber and a pending require are Conflict. No row stays
  additive (Cluster 375 shape). Store test `soundcheck_gate`.
- **385.3 (#826) — REST + MCP.** `PUT`/`GET`/`DELETE
  /threads/:id/soundcheck` plus `PUT …/soundcheck/requirement`. MCP
  `set_soundcheck` / `get_soundcheck` / `require_soundcheck` /
  `clear_soundcheck`. Writes = `thread:transition`; reads =
  `workspace:read`. Full new-route preflight + both sorted MCP
  contracts.
- **385.4 (#827) — e2e.** Auth-enabled minted tokens. Vacuous GET is
  green; unskilled PUT is 400; require then close is 409; amber 409;
  owner pass 409; skilled third-party green pass lands. Fail stays red
  even when `land=green` is requested. MCP `get_soundcheck` matches.
  Close via REST + `store.transition_thread` (P1.1d owns the MCP twin).
- **385.5 (#831) — this retro + the doc-close.** Strike Open Work #25 fully.
  Cluster 384 / #26–28 / P1.1d are other agents' work.

## Decisions

- **Additive, like Cluster 375.** No row → close as before. Presence of
  a row arms the gate. Forcing every thread through Soundcheck would
  break every existing close.
- **Skill + SoD on standing, not on the wire shape.** Soundcheck writes
  `{kind, status, artifact_sha?, land}`. The room decides landable:
  pass + green + `soundcheck` skill + recorder ≠ owner/assignee.
- **Amber is not a land.** Flags-then-still-engages stays amber even
  when status is pass. Fail is always red (`resolve_land` wins over a
  requested green).
- **Room holds the pointer; Soundcheck owns test execution.** Not a CI
  product and not a judge panel in the room. Pi may still run
  BullshitBench; that dataset is not this cluster.
- **No MCP `transition_thread`.** That is P1.1d (Cluster 384). The
  store FSM is the shared gate; REST `POST /threads/:id` already calls
  it.

## Surprises

- **Cluster 384 was claimed mid-stack.** P1.1d took 384 (#816/#823).
  Blocked-reason took 386; run-lineage took 387. Soundcheck moved to
  385. Titles and bodies were rewritten; #819/#821 stay on `main`.
- **SQLite `EXISTS` is `i64` 0/1.** `query_scalar` does not bind a
  bool. Same lesson as other sqlite EXISTS gates.
- **Fail + requested green is still red.** `resolve_land` is the
  product: the FSM will not `closed` on accepted nonsense.

## Test evidence

- Types: `resolve_land` / `is_qualifying_pass` / `soundcheck_standing`
  (vacuous green; pending red; skilled third-party amber stays amber;
  owner pass not landable; fail always red).
- Store, both backends (`soundcheck`): require / set / standing /
  clear; unskilled write `InvalidInput`. `soundcheck_gate`: pending /
  amber / owner / fail block `closed`; qualifying green pass lands.
  `review_gate` still green (375 composition untouched).
- MCP: `soundcheck_tools_require_set_get_and_clear` + catalog /
  capability-map contracts.
- Server: `openapi_e2e` + `http_capability_matrix_e2e`;
  `soundcheck_e2e` (auth-enabled) — HTTP close-gate + MCP standing.
- Clippy `-D warnings` on types/store/server/mcp.

## Forward look

**Cluster 385 is complete.** Row #25 is closed (383 composition + 385
pointer). The room will not `closed` on a required Soundcheck that is
pending, amber, fail, or an implementer self-pass.

Deferred (follow-ups): a `SoundcheckRecorded` event so a waiter reacts
without polling; `/ui` land chip; auto-require on a recipe / thread
type (opt-in). P1.1d (`transition_thread` MCP) was not taken — Cluster
384. Open Work #26–28 (blocked-reason, occupancy follow, run lineage)
were not touched.

**Not this cluster:** Wave 3. Soundcheck still owns test execution
outside this repo.

## Acknowledgements

Four impl PRs (#819 store+types → #821 FSM → #826 REST+MCP → #827 e2e)
+ this retro (#831). Coordination: keep #819/#821 on `main`; do not fight
other agents on Open Work #26–28 or the P1.1d / blocked-reason /
run-lineage PRs.
