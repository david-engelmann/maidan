# Cluster 26.0 retro — Product completion gate

> Closing wave for Cluster 26.0 · target tag `v26.0.0` (shipped with ladder PR #198).

Cluster 26.0 documented critical-path completeness and added a lightweight integration gate.

## What shipped

- **PR #198** (`0cffd8f`) — [[Product Completion Checklist]], `product_completion_gate_e2e`
  (health, `/ui`, `/mcp/streamable`, `/a2a/v1/rpc` presence).

## What was deferred

| To | What | Why |
|----|------|-----|
| [[Remaining Work]] | Exhaustive “no stubs” proof | Gate is smoke, not full matrix. |
| Post-26 | Compose federation + Helm e2e in gate | CI cost / setup. |
| Post-26 | Positive capability matrix for every route | 22.0 covered denials only. |

## Surprises

- A2A empty body returns non-404 (422) — gate asserts endpoint exists, not success.

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| Documented product completion checklist | `v26.0.0` |
| Completion gate e2e smoke | `v26.0.0` |

## Forward look

**Cluster 27.0** in same PR. Ladder closes at **`v27.0.0`**.

## Acknowledgements

- Maintainer merge #198.
