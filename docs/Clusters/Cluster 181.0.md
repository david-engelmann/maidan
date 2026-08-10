# Cluster 181.0 — correctness: one EventKind parser, round-trip guarded

**Theme:** Arc A (security & correctness), part 3 — remove the drift-prone
per-backend `EventKind` parsers that caused Cluster 171's silent event loss, and
lock the survivor with a round-trip guard.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v181.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Store `parse_kind` (both backends) delegates to `EventKind::parse` — no per-backend copy | `maidan-store/src/{sqlite,postgres}/events.rs` |
| `EventKind::ALL` + round-trip guard test (compile-time tripwire on new variants) | `maidan-types/src/events.rs` |

## Why

Adding an `EventKind` used to require editing **three** parsers in lockstep:
`EventKind::parse` in `maidan-types`, and a *separate, identical* `parse_kind`
in **each** of `store/postgres/events.rs` and `store/sqlite/events.rs`. The store
copies are read back inside `append`: the row is inserted with
`event.kind().as_str()`, then `row_to_stored` re-parses the returned `kind`
column via `parse_kind`. If a variant is missing from a store copy, the read-back
returns `Err`, `append` never reaches `tx.commit()`, and **the event silently
rolls back** — exactly the Cluster 171 bug (`thread_assignment_changed` shipped
in `EventKind::parse` but not in the store copies; caught late).

A "parity guard test" over three copies would only *detect* drift. Removing the
duplication *prevents* it.

## The fix

- Both store `parse_kind` functions now delegate to
  `maidan_types::EventKind::parse(s).ok_or_else(|| StoreError::InvalidInput(…))`.
  The wire-form mapping (snake_case ↔ variant) lives in exactly one place; the
  store keeps only the thin `Option → Result<_, StoreError>` adapter its call
  sites expect.
- `EventKind::ALL: &'static [EventKind]` lists every variant, and a new
  `kind_tests` module asserts `parse(kind.as_str()) == Some(kind)` for each. The
  test body contains an **exhaustive `match`** (no wildcard) over the variants,
  so adding a variant to the enum fails to compile there until it's listed —
  the reminder to add it to `ALL`, where the round-trip check then covers it.
  This guards the one remaining parser against `as_str`/`parse` drift.

## Exit criteria

- No `EventKind` string mapping exists outside `maidan-types`; the round-trip
  guard passes; a new variant trips a compile error in the guard test — **met**.
- `v181.0.0` tagged.

## Verification & limits

- `maidan-types`: `all_variants_round_trip`, `all_lists_each_variant_once`,
  `unknown_kind_does_not_parse`. Existing store integration tests continue to
  exercise the real `append`→read-back path for the common kinds.
- Limit: the tripwire is a compile error *in the guard test's match*, which
  points the author to `ALL` by comment rather than mechanically forcing the
  `ALL` edit. A fully mechanical proof would need a proc-macro / `variant_count`
  (unstable); the tripwire + single-parser collapse is the pragmatic close.

## References

- [[Retros/Cluster 181.0]]; `maidan-types/src/events.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc A). Bug precedent: [[Retros/Cluster 171.0]].
