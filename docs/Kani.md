# Kani proofs

Bounded proofs, run locally. They are not a CI job, and nothing here changes
a workflow.

| Property | Where | Bound |
|---|---|---|
| A subscribe cursor is too old only when its next id is strictly behind the oldest retained row. A fresh cursor, an empty log, an adjacent resume, and `i64::MAX` (no next id) are not. | `crates/maidan-types/src/cursor.rs` | every `i64`, every floor |
| Catch-up is allowed exactly when that cursor is not too old. | same, calling `catch_up_allowed` | every `i64`, every floor |
| A too-old body always says to refetch. | `CursorTooOld::new` | every `i64` pair |
| The room high-water is never negative. | `RoomLsn::from_max_id` | every `i64` |

The harnesses are `#[cfg(kani)]`. A normal build, clippy, and the tests do
not compile them. `cfg(kani)` is an expected cfg on `maidan-types`, so
`-D warnings` stays quiet.

## Run

Install once. The verifier bundle is about 500 MB under `KANI_HOME`
(default `~/.kani`) and it downloads its own Rust nightly beside that:

```bash
cargo install --locked kani-verifier
export KANI_HOME="${KANI_HOME:-$HOME/.kani}"
cargo kani setup
scripts/kani-proofs.sh
```

One harness: `cargo kani -p maidan-types --harness a_pruned_gap_is_the_only_cursor_that_is_too_old`.
