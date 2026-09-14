# Cluster 384 retro — P1.1d: MCP `transition_thread` twin of the REST FSM

Cluster 384 closes the last MCP write-path gap from the 2026-09-10
audit. REST has had `transition_thread` / `transition_thread_with_event`
since Cluster 208 (SoD in 355, required-reviewers close-gate in 375,
critical→`request_changes` in 383). MCP had no tool that called that
path. An MCP-only agent could claim, assign, and set a result, but could
not advance the FSM.

This is **not a new Wave number** and **not a land-gate**. The default
decision was gap, not "MCP must never close." Cluster 383's "P1.1d,
intentional" meant *that* cluster tested close via
`store.transition_thread` — it did not decide the tool must stay
impossible. Wave 2 #22 review tools under `thread:transition` did not
close this: that capability is the grant label for disposition changes,
not a call to `transition_thread_with_event`.

One impl PR + this retro. Both target `main` (Cluster 383's
`base_ref_deleted` lesson).

## What shipped

- **384.1 (#816)** — MCP `transition_thread` (`thread:transition`).
  Args `{thread_id, actor_id, action}` (`start_review` / `close` /
  `archive`). Calls `transition_thread_with_event` + `publish_stored`.
  Same store gates as REST: SoD (claimer cannot land owned work),
  required-reviewers close-gate, unresolved `refutes`, Cluster-383
  critical composition. Terminal transitions emit Cluster-222
  `ThreadReady` for newly-ready dependents (derived, best-effort).
  Resource subscribers get the thread/channel/workspace URIs. Unknown
  actions and gate refusals are `InvalidParams`. `maidan-fsm` moved
  from a mcp *dev*-dependency to a runtime dependency so the non-test
  crate can parse `ThreadAction`.
- **384.2 (#823)** — this retro + the doc-close (mark P1.1d ✅ FIXED,
  Integration landing note, Protocols tool count, Architecture FSM row,
  Capability Map `thread:transition`). After rebase onto Cluster 385,
  the published count is **155** (384's twin + 385's four LandGate
  tools).

## Decisions

- **Gap, not a forever land-gate.** The FSM already owns SoD / close /
  reviewers. Hiding the twin from MCP would only force agents onto REST
  or a raw store call. Identical rules, no bypass.
- **`actor_id` is an argument**, matching REST `TransitionThread` and
  MCP `assign_thread` / `unassign_thread`. Bearer = act-as-any.
- **Do not fold close into the waiter loop.** A waiter that claimed the
  work is the party SoD forbids from landing it once an owner is set.
  Landing is an owner/reviewer (or un-owned) action.
- **No LandGate work.** Row #25 remainder (pointer + green/amber/red)
  and Wave 2 #26–28 / Wave 3/4 are untouched.

## Surprises

- **`maidan-fsm` was test-only on `maidan-mcp`.** Lib tests compiled
  because they already imported `ThreadAction`. `cargo clippy` on the
  non-test crate failed until the dep moved.
- **A helper closure over `call_tool` does not compile.** The future
  holds `&AuthContext` across the async boundary. Inline the calls.

## Test evidence

- **DoD:** `transition_thread_tool_advances_fsm_and_honors_gates` —
  unknown action → `InvalidParams`; `start_review` → `in_review` +
  `ThreadStateChanged` on the bus; claimer `close` on an owned thread
  → SoD `InvalidParams`; `k=1` with no approval → close-gate
  `InvalidParams`; third-party approve → owner `close` → `closed`.
- Cluster 383's `critical_result_tool_blocks_close_until_a_human_approves`
  now closes via the tool (was `store.transition_thread`).
- All **87** mcp lib tests + both contract-sync tests +
  `mcp_capability_matrix_e2e`. `clippy -p maidan-mcp --all-targets -D
  warnings` + `fmt --check` clean.

## Forward look

**P1.1d is closed.** The MCP write path now matches REST for message,
social, assignment, **and** FSM transition.

**Not this cluster:** Wave 2 #26–28, Wave 3/4. Cluster 385 (LandGate
pointer + land vocabulary, #819/#821/#826/#827/#831) is independently
on `main`; this retro does not re-close it.

## Acknowledgements

384.1 (#816) + this retro (#823). Both PRs target `main`.
