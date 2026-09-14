# Cluster 389 retro — OSS hygiene: de-internalize / land-gate

Maidan is a **public OSS** room. Cluster 385 shipped a close-gate whose
semantics were right (pointer + pass/fail + green/amber/red land
vocabulary + FSM refuse-`closed` without a qualifying pass from a
gate-skilled member ≠ owner/assignee) but whose public surface named an
internal product. Cluster 389 renames that surface and scrubs the rest
of the tree of internal product names.

**386** is blocked-reason. **387** is run-lineage. **388** was left
unused. This work is **389**.

One impl PR + this retro. Every PR targets `main`.

## What shipped

- **Vocabulary.** Chosen name: **`land_gate`** / `LandGate` /
  `kind: "land_gate"`. One coherent set:
  - types: `LandGateStatus`, `LandGatePointer`, `LandGateStanding`,
    `LAND_GATE_SKILL` (`"land_gate"`), `LAND_GATE_KIND` (`"land_gate"`)
  - store: `maidan_thread_land_gate` (pg 0088 / sqlite 0087 rewritten
    in place — unused-repo greenfield, no dual-write)
  - REST: `PUT`/`GET`/`DELETE /threads/:id/land-gate` +
    `PUT …/land-gate/requirement`
  - MCP: `set_land_gate` / `get_land_gate` / `require_land_gate` /
    `clear_land_gate`
- **Gate behavior unchanged.** Require arms; no row is vacuous green;
  amber is not a land; fail is always red; qualifying pass = green +
  `land_gate` skill + recorder ≠ owner/assignee.
- **Result-kind / waiter envelope.** Examples are now
  `example.review.result/1` (and `example.plan.result/1`,
  `example.novel.result/9`). The frozen waiter schema is
  `maidan.waiter.result/1`. The delivery backlink field is `view_url`
  (was an internal name). Fixture:
  `crates/maidan-types/tests/fixtures/waiter_result_v1.json`.
- **Test selectors.** GitHub allowlist / outbox fixtures use
  `example/repo`, not an internal repo.
- **Docs.** Integration / Result Delivery / Architecture / Open Work
  describe a generic room + external verifier / waiter / coding agent.
  Historical retros rewritten to the same vocabulary. Raspberry Pi
  (`docs/Pi.md`) is unchanged — that is the public ARM64/edge page.

## Decisions

- **`land_gate` over `quality_gate`.** The land vocabulary
  (green/amber/red) already lived on the pointer; the gate *is* the
  land. One word.
- **Replace names cleanly.** No backwards-compat shims, no dual-write,
  no alias for the old internal name. Greenfield unused-repo.
- **Do not invent a second product.** The room holds the pointer; an
  **external verifier** records pass/fail. Not a CI product and not a
  judge panel.
- **Keep Raspberry Pi.** `docs/Pi.md` and "Pi/edge" in the operator
  gate are the public ARM64 install path, not an internal world.

## Surprises

- **`view_url` was a load-bearing wire key**, not just a comment. The
  waiter parser reads it by name; the fixture lock and every delivery
  e2e carried the old key. Renaming it is a producer-visible grammar
  change — accepted because David waived backwards-compat.
- **MCP tool-name sort moved.** The old get-* tool sat next to
  `get_spawn_budget`; `get_land_gate` sorts next to `get_inbox`. Both
  contract files were re-sorted.
- **Line-broken "owns test execution"** and leftover **Pi-as-product**
  phrasing (secret-ref "resolves at exec", Cluster 385 "owns test
  execution", Cluster 374 private-path cite) survived the first pass.
  Phrase cleanup has to match across newlines and historical retros.

## Test evidence

- Types: `resolve_land` / `is_qualifying_pass` / `land_gate_standing`
  (vacuous green; pending red; skilled third-party amber stays amber).
- Store, both backends (`land_gate` + `land_gate_gate`).
- MCP: `land_gate_tools_require_set_get_and_clear` + catalog /
  capability-map contracts (sorted).
- Server: `openapi_e2e` + `http_capability_matrix_e2e`;
  `land_gate_e2e`; waiter fixture lock; result-kind / delivery e2es
  with `example.*` / `maidan.waiter.result/1`.
- Clippy `-D warnings` on types/store/server/mcp.

## Forward look

**Cluster 389 is complete.** The public surface has no internal
product names. The land-gate still works. Wave 2 #26 (occupancy
follow) / #28 remainder (manager digest) / Wave 3 are not this work.

Deferred (unchanged from 385): a `LandGateRecorded` event so a waiter
reacts without polling; `/ui` land chip; auto-require on a recipe /
thread type.

## Acknowledgements

One hygiene PR + this retro. Coordination: 386/387 already claimed
those numbers; 388 left unused on purpose.
