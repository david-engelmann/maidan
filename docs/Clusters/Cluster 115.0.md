# Cluster 115.0 — Module split + `unwrap()` purge

**Theme:** Close Phase XXI by hardening error handling (no panicking `unwrap`/`expect` in library code, enforced) and breaking the two largest flat files into domain modules.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXI · tag **`v115.0.0`** (closes the phase).

**Predecessor:** the "no `unwrap()` in library code" convention (CLAUDE.md); the flat `routes.rs` / `tools.rs`.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Robustness** | Drive non-test `unwrap()`/`expect()` in `crates/*/src` to zero, fixing each by its real nature (poison recovery, `unreachable!`, graceful fallback, or a single documented invariant `panic!`). |
| **CI** | A clippy gate (`-D clippy::unwrap_used -D clippy::expect_used`, scoped `--lib --bins`) so it stays at zero. |
| **Structure** | Split `routes.rs` (1617 lines) and `tools.rs` (1368 lines) into domain-organized module directories, preserving all public paths via `mod.rs` re-exports. |

## Non-goals

- Gating explicit `panic!` / `unreachable!` / `todo!` — out of scope; would require revisiting genuine invariants.
- Any behavior change in the split — pure reorganization.

## PR ladder (actual)

| # | Title |
|---|--------|
| 115.0.1 | `refactor: purge non-test unwrap()/expect() from src; enforce via clippy` (#316) |
| 115.0.2 | `refactor(server): split routes.rs into domain modules` (#317) |
| 115.0.3 | `refactor(mcp): split tools.rs into domain modules` (#317) |
| 115.0.retro | `docs(retro): Cluster 115.0 + v115.0.0 tag prep` |

## Exit criteria

- `routes.rs` and `tools.rs` split by domain — **met**.
- Zero non-test `unwrap()`/`expect()` in `crates/*/src` — **met**.
- Clippy lint added to enforce — **met** (lint job step, `--lib --bins`).
- `v115.0.0` tagged after retro — closes Phase XXI.

## Ordering & risks

- **Purge before split:** landing the gate first (115.0.1) means the splits
  (.2/.3) are checked against it automatically — a moved-in `unwrap` can't slip through.
- **Risk — split breaks wiring:** mitigated by `pub use` re-export (public
  paths unchanged) and independent re-verification (clippy gate + lib tests +
  `mcp_e2e`/`http_crud_e2e`/`reactions_pins_e2e`/capability-matrix e2e).

## References

- [[Clusters/Product Ladder 102+]] Phase XXI
- [[Retros/Cluster 115.0]]
