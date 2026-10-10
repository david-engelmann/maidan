# Kani proofs

Bounded proofs, run locally. They are not a CI job, and nothing here changes
a workflow.

| Property | Where | Bound |
|---|---|---|
| A subscribe cursor is too old only when its next id is strictly behind the oldest retained row. A fresh cursor, an empty log, an adjacent resume, and `i64::MAX` (no next id) are not. | `crates/maidan-types/src/cursor.rs` | every `i64`, every floor |
| Catch-up is allowed exactly when that cursor is not too old. | same, calling `catch_up_allowed` | every `i64`, every floor |
| A too-old body always says to refetch. | `CursorTooOld::new` | every `i64` pair |
| The room high-water is never negative. | `RoomLsn::from_max_id` | every `i64` |
| A request outside an installation grant is not contained. Success means every asked capability was granted. | `first_not_held`, called by `validate_subset` | every subset of `workspace:read` and `token:admin` |
| Attenuation does not amplify. An asked capability that was not held is refused. | `first_not_held`, called by `attenuate` | the same two |
| A repeated ask is kept once, in first-seen order. | `unique_strs`, called by `attenuate` | `workspace:read` twice |
| Every known capability is work (delegatable) or authority, not both. | `is_delegatable` | every index into the known list |

The harnesses are `#[cfg(kani)]`. A normal build, clippy, and the tests do
not compile them. `cfg(kani)` is an expected cfg on `maidan-types` and
`maidan-auth`, so `-D warnings` stays quiet.

`validate_subset` and `attenuate` call `first_not_held`. The granted set and
the error strings are unchanged. The proofs do not change minting.

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
