# Cluster 115.0 retro — Module split + `unwrap()` purge

> Tag **`v115.0.0`**. Fifth and final cluster of **Phase XXI (correctness & coverage)** — closes the phase.

## What shipped

- **Non-test `unwrap()`/`expect()` purged from `crates/*/src`** (115.0.1) — 25
  sites, each fixed by its true nature rather than a blanket conversion:
  - Lock guards (presence ×8, rate-limit, metrics-hydrate) recover on poison
    via `unwrap_or_else(PoisonError::into_inner)` instead of panicking.
  - Mathematically-infallible calls (HMAC-SHA256 any-key-length ×3, the
    `EPOCH` RFC3339 constant) use `unwrap_or_else(|_| unreachable!(...))`.
  - The const `application/problem+json` content type → `HeaderValue::from_static`.
  - Dynamic header parse (`x-artifact-kind`) and a guarded `Vec::pop` → `if let`.
  - Infallible A2A stream-frame serialization → `unwrap_or(Value::Null)`.
  - Best-effort metrics init → `match` + `tracing::error!` fallback
    (Prometheus-only) instead of `expect` at startup.
  - The `subscribe_resume_secret` construction invariant → an explicit
    `panic!` (no `Result` to thread through a `&[u8]` return).
  - The codegen bin → `main() -> io::Result<()>` with `?`.
  - `presence::register_connection` subscribes to the room while it is still
    in hand, removing a fallible re-lookup entirely.
- **A clippy gate keeps it at zero** (115.0.1): a new lint step
  `cargo clippy --workspace --lib --bins -- -D clippy::unwrap_used
  -D clippy::expect_used`. Scoped to `--lib --bins` so it covers `crates/*/src`
  exactly — `#[cfg(test)]` modules aren't compiled there, and `tests/` +
  `benches/` are excluded, so tests keep using `unwrap()` freely.
- **`routes.rs` (1617 lines) split by domain** (115.0.2) into `routes/` with
  `mod.rs` re-exporting domain submodules (`workspace`, `member`, `channel`,
  `thread`, `message`, `social`, `artifact`, `reference`, `search`, `token`)
  via `pub use <domain>::*;`. Every `crate::routes::<handler>` path and the
  cross-module `pub(crate)` helpers resolve unchanged.
- **`tools.rs` (1368 lines) split by domain** (115.0.3) into `tools/` with
  `mod.rs` keeping the entry points (`required_capability`, `dispatch`) and
  re-exporting `catalog`; submodules `channel`, `message`, `social`,
  `artifact`, `thread`, `reference`, `search`, `automation`, `catalog`.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Phase XXII (116) | Batch embedding pipeline | Phase XXI is closed; the ladder moves to search/indexer-at-scale. |
| (future)     | Apply the unwrap/expect gate to bins of other crates, or widen to `panic`/`todo` lints | The gate covers `--lib --bins` for the whole workspace already; tightening to forbid explicit `panic!` would require revisiting the genuine invariants (e.g. `subscribe_resume_secret`). |

## Surprises

- **The raw `unwrap`/`expect` count was ~3× the real one.** A naïve grep of
  `crates/*/src` found dozens, but the bulk were inside inline `#[cfg(test)]`
  modules (including the envelope tests added in Cluster 114). Scoping the
  clippy gate to `--lib --bins` — which doesn't compile `#[cfg(test)]` —
  turned out to *be* the precise definition of "non-test in src", so the gate
  and the requirement are the same thing rather than two approximations.
- **The split was invisible to callers.** Because `app.rs` and the other
  modules reach handlers through `crate::routes::*` / `crate::tools::*`, a
  `mod.rs` of `pub use <domain>::*;` re-exports preserved every path with zero
  edits outside the split files — the diff is almost entirely file moves.

## Decisions

- **Fix each `unwrap` by its true nature, not a blanket rule.** Poison
  recovery for locks (an in-memory roster shouldn't crash the server),
  `unreachable!` for the provably-infallible, graceful fallback for
  best-effort observability, and an explicit `panic!` reserved only for a
  genuine construction invariant. This keeps behavior honest rather than
  trading one panic for a silent wrong default.
- **Gate scoped to `--lib --bins`, not `--all-targets`.** `--all-targets`
  drags integration tests and benches under the lint, which the repo
  convention explicitly allows to `unwrap`. `--lib --bins` matches
  "crates/*/src" precisely. No [[Decisions]] change; consistent with the
  existing "no `unwrap()` in library code" rule, now enforced.
- **Re-export split over moving the router.** Keeping the public module path
  stable via `mod.rs` re-exports was lower-risk than rewiring `app.rs`'s
  ~55 route registrations.

## Capability table extension

| Capability | Where |
|------------|-------|
| No non-test `unwrap()`/`expect()` in `crates/*/src` (clippy-enforced) | `.github/workflows/ci.yml` (lint job) |
| Domain-organized HTTP route modules | `crates/maidan-server/src/routes/` |
| Domain-organized MCP tool modules | `crates/maidan-mcp/src/tools/` |

## Risks identified + mitigated

- **Panic surface in library code.** The lock/HMAC/serialize paths no longer
  panic on poison or impossible errors; the only remaining `panic!` is a
  documented construction invariant.

## Risks identified + still open

- **`panic!`/`unreachable!` are not lint-gated.** The gate forbids
  `unwrap`/`expect` but not explicit `panic!`/`unreachable!`. A future cluster
  could widen it, but that means re-examining the genuine invariants first.

## Forward look

**Phase XXI (correctness & coverage) is complete.** The ladder moves to
**Phase XXII — Search & indexer at scale (Clusters 116–118)**, opening with
**Cluster 116 — batch embedding pipeline** (batch embed calls with
backpressure; large-workspace backfill on a separate queue; bounded
indexer-lag metric).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. The two file splits
were executed mechanically and independently re-verified (clippy gate + lib
tests + dispatch/route e2e) before merge.
