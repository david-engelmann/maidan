# Cluster 112.0 — FSM property tests

**Theme:** Prove the thread lifecycle FSM and the hierarchical parent/child rank rule hold for *arbitrary* inputs, not just the hand-picked happy edges.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXI · tag **`v112.0.0`**.

**Predecessor:** FSM engine from Cluster D ([[Roadmap]] `v0.3.0`); hierarchical rank rule in `maidan-fsm/src/hsm.rs`.

---

## Problem

`maidan-fsm` is the authority on legal thread-state transitions — `apply((state, action))` and the HSM rule `parent_allows_transition(parent, child_to)`. It carried only per-edge `#[cfg(test)]` unit tests (the three legal edges + a couple of illegal cases). Nothing covered the combinatorial `(state, action)` space, arbitrary action sequences, or arbitrary parent/child tree shapes — exactly where an illegal transition or a child outrunning its parent would corrupt thread state. For a correctness phase, the FSM is a natural target for property-based testing because it is small, pure, and total.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Transitions** | `proptest` proving `apply` succeeds on exactly the legal edges (vs an independent spec), advances rank by one, and treats `Archived` as terminal. |
| **Sequences** | Rank monotonicity under arbitrary action sequences (no regress, no overflow, `Archived` absorbing). |
| **Hierarchy** | The HSM rank ceiling for every `(parent, child_to)`, and the tree-wide composition: locally-valid edges ⟹ no descendant outruns any ancestor. |
| **Parsing** | `parse` accepts exactly the known wire strings; `parse ∘ as_str` round-trips. |

## Non-goals

- Changing FSM behavior — tests only.
- FSM persistence / HTTP wiring (lives in `maidan-store` / `maidan-server`; its backend parity is Cluster 113).

## PR ladder (actual)

| # | Title |
|---|--------|
| 112.0.1 | `test(fsm): property tests for thread FSM + hierarchical rank rule` (#310) |
| 112.0.retro | `docs(retro): Cluster 112.0 + v112.0.0 tag prep` |

## Exit criteria

- `proptest` proving only legal `(state, action)` edges succeed, `archived` terminal, and the HSM child-rank ≤ parent invariant holds for arbitrary trees — **met** (8 properties).
- `v112.0.0` tagged after retro.

## Ordering & risks

- **Independent of [[Clusters/Cluster 111.0]]** — both pure-logic crates; 111 shipped first, 112 second.
- **Risk — tautological table test:** mitigated by keeping the `spec_next` edge table independent of `apply`'s own `match`.
- **Risk — proptest flake / slow shrink:** the state space is tiny (4 states, 3 actions, trees ≤ 11 nodes); cases run in well under a second.

## References

- [[Clusters/Product Ladder 102+]] Phase XXI
- [[Retros/Cluster 112.0]], [[Architecture]]
