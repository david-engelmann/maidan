# Cluster 383 retro — Wave 2 #25 composition: critical waiter findings → Cluster-375 `request_changes`

Wave 2 #25 asked for a Soundcheck gate pointer and a green/amber/red land-gate
vocabulary so the FSM will not `closed` on accepted nonsense. The 2026-09-12
composition found a stronger slice that needs **no new gate machinery**: a
delivered `pi.review.result/1` whose `findings` contain any `critical` **is**
a `request_changes` from a review-skilled agent. Feed it into the
**Cluster-375 required-reviewers close-gate** and the room refuses `closed`
until a human resolves.

This cluster is **only that adapter**. The Soundcheck `{kind:"soundcheck",…}`
pointer and the green/amber/red vocabulary stay on row #25.

Three impl PRs (383.1–383.3) + this retro. #810 and #812 died
`base_ref_deleted` when their parents squash-merged; #811 and #814 are
the rebuilds from `origin/main`. This retro targets `main` only.

## What shipped

- **383.1 (#809) — types + store.** `review_decision_from_waiter` maps a
  reviewed `pi.review.result/1` with any exact `severity == "critical"` to
  `ReviewDecision::RequestChanges` (else `None`). `result_kind` and
  `severity` stay free strings, not enums. Severity is walked on the **raw**
  `findings` array — a critical finding without `file` / `body` /
  `line_range` still counts (380's usability filter would skip it).
  `ReviewStore::apply_critical_review_decision` skill-checks `REVIEW_SKILL`
  (`"review"`) and upserts `request_changes` with `CRITICAL_REVIEW_NOTE`.
  383.1 did **not** set `k` — writing the decision alone is not a veto.
- **383.2 (#811, replaces closed #810) — arm `k=1` + `ThreadResultSet`.**
  `request_changes` is not a veto when `required_count == 0`
  (`approvals_met` is then true). The adapter now `set_requirement(thread, 1)`
  when no requirement exists; an existing `k` is left alone. The
  `ThreadResultSet` arm calls `arm_critical_review` after the 379 delivery
  arm. Empty `deliver_to` still arms — the room blocks the land even when
  nothing is posted externally.
- **383.3 (#814, replaces closed #812) — write-path + e2e.** `PUT /threads/:id/result` and MCP
  `set_thread_result` call `apply_critical_review_decision` after the upsert
  so a PUT is immediately visible (the 383.2 bus consumer is every-replica /
  replay). HTTP e2e: critical blocks `close` (409) until a third-party human
  `approve`. Warning-only does not arm `k`. MCP twin proves the same gate
  via `store.transition_thread` (there is no MCP `transition_thread` tool —
  P1.1d, intentional).
- **383.4 — this retro + the doc-close.** Strike the #25 composition.
  Keep the Soundcheck pointer / green-amber-red as the remaining row.

## Decisions

- **No new gate.** Cluster 375 already refuses `closed` when `k` is unmet.
  The adapter writes a decision and, if needed, arms `k=1`. Owner/assignee
  approvals still do not count (SoD). A third-party human `approve` unblocks.
- **Never auto-approve.** A clean re-review does not land. The adapter only
  writes `request_changes`.
- **Exact strings, not enums.** `result_kind == "pi.review.result/1"`,
  `status == "reviewed"`, `severity == "critical"`. A warning, a plan
  result, or a failed review is inert.
- **Skill-gated.** The producer must have declared `review`. An unskilled
  member writing the same envelope is a no-op.
- **GitHub `event` stays `COMMENT`.** Cluster 380 does not APPROVE or
  REQUEST_CHANGES on the PR. The room gate is the land decision; the
  external review is still comments on the diff.
- **Fire from the result, not only from a successful delivery.** Empty
  `deliver_to` is a supported outcome. The write path (383.3) plus the bus
  consumer (383.2) both arm so a single replica and every replica stay
  consistent.

## Surprises

- **`request_changes` is not a veto.** `approvals_met` is true whenever
  `required_count == 0`. 383.1 wrote the decision and close still
  succeeded. Arming `k` is the load-bearing half of the product.
- **A stacked PR targeting a deleted base is closed, not retargeted.**
#810 died the moment #809 squash-merged (`base_ref_deleted` /
CONFLICTING). #812 died the same way after #811. #811 and #814 are the
rebuilds onto `origin/main`. Same lesson as 379.4 / #790, 382.2 / #795,
380.2, 381.3.
- **The bus consumer races a same-process PUT→close.** 383.2 is correct
  for multi-replica replay; 383.3's write-path arm is the product contract
  (immediately visible), not just a test convenience.
- **Clippy `doc_lazy_continuation` on a leading `+`.** The 383.1 store
  comment used a markdown list that started a continuation with `+`;
  reworded to "plus" before #809 merged.

## Test evidence

- Types: `review_decision_from_waiter` — critical + reviewed +
  `pi.review.result/1` → `RequestChanges`; warning-only / wrong kind /
  not-reviewed → `None`. Fixture lock
  `the_authoritative_fixture_is_a_critical_request_changes`.
- Store, both backends (`review_from_result`): skilled + critical →
  `request_changes` + `k=1` → close `Conflict`; unskilled no-op; existing
  `k=2` left alone; owner self-approve does not unblock; third-party human
  approve unblocks. `review_gate` / `reviews` still green.
- Server: `critical_review_e2e` (auth-enabled, minted tokens) — PUT
  critical → `review-status` `k=1` / not met → `start_review` then `close`
  409 → human `approve` → `close` 200 `closed`. Warning-only leaves `k=0`.
- MCP: `critical_result_tool_blocks_close_until_a_human_approves` —
  `set_thread_result` + `get_review_status` + `submit_review`; close via
  the store FSM.

## Forward look

**Cluster 383 is complete.** The #25 composition (critical →
`request_changes` → Cluster-375 close-gate) is shipped. pi reviews → the
room blocks the land → a human decides.

**Remaining on row #25:** the Soundcheck `{kind:"soundcheck",
status:pass|fail, artifact_sha?}` pointer and the green/amber/red
vocabulary. Not a CI product / a judge panel in the room.

Deferred (follow-ups): bumping an already-met `k` when a late critical
arrives; auto-approve on a clean re-review (declined); a `ReviewSubmitted`
event so a waiter reacts without polling (Cluster 375 deferral). P1.1d
(`transition_thread` MCP) was not taken.

**Not this cluster:** Wave 3. The Cluster-380 GitHub review `event` is
still `COMMENT`.

## Acknowledgements

Three impl PRs (#809 store+types → #811 arm `k` + `ThreadResultSet` →
#814 write-path + e2e) + this retro (#815). #810 and #812 closed
(`base_ref_deleted` after the parent squash). #808 (Cluster 381 retro)
merged onto `main` before 383.2 rebuilt.
