# Cluster 181.0 retro — one EventKind parser, round-trip guarded

> Tag **`v181.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc A (security & correctness), part 3. **Removes a bug class, not just a bug.**

## What shipped

- The two store `parse_kind` copies (`postgres/events.rs`, `sqlite/events.rs`)
  now delegate to the single `maidan_types::EventKind::parse`. The snake_case ↔
  variant mapping exists in exactly one place; each store keeps only a two-line
  `Option → Result<_, StoreError>` adapter.
- `EventKind::ALL` + a `kind_tests` module: `all_variants_round_trip` asserts
  `parse(as_str())` for every variant, with an exhaustive `match` (no wildcard)
  as a compile-time tripwire when a variant is added.
- `EventKind` gained `Copy` (fieldless enum; `as_str(self)` was already by-value).

## Surprises

- **The scoped task was "a parity guard test over three parsers"; the better
  move was to delete two of the three.** A guard only *detects* drift after a
  variant is added; delegating to one parser *prevents* it. The store never
  needed its own mapping — the copies were incidental duplication, not a design
  choice. Cluster 171's silent event loss is now structurally impossible: the
  store has no parser to forget.
- **`Copy` was the unlock for ergonomic iteration.** `for &kind in ALL` +
  `kind.as_str()` (by-value) + `{kind:?}` all need `Copy`; the enum is fieldless
  so it's free, and it simplifies the by-value `as_str`/`parse` signatures that
  were already written as if it were `Copy`.

## Decisions

- **Collapse over guard.** Kept a thin store `parse_kind` (rather than inlining
  `EventKind::parse` at the two call sites) so the `StoreError` adapter + the
  bug-precedent comment live in one spot per backend, and call sites are
  unchanged.
- **Tripwire, not proc-macro.** A fully mechanical "ALL is complete" proof needs
  `variant_count` (unstable) or a derive macro; the exhaustive-match tripwire +
  single-parser collapse is the pragmatic close for a 17-variant enum.

## Capability table extension

| Change | Where |
|--------|-------|
| Single EventKind wire-form parser (store delegates) + round-trip guard | `maidan-types` + `maidan-store/src/{sqlite,postgres}/events.rs` |

## Risks identified + still open

- **Net risk-reducing, zero behaviour change** — the delegated mapping is
  character-identical to the deleted copies; the store still errors on an unknown
  kind with the same message. Adding `Copy` is a pure widening (may let clippy
  flag now-redundant `.clone()` on an `EventKind`, swept by the all-targets pass).

## Forward look

Arc A continues: audit-log coverage (182), default-on rate limits + body cap
(183), dual-write atomicity — the deeper cousin of this bug: `publish()` commits
the domain row then appends the event separately (184).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Bug precedent:
[[Retros/Cluster 171.0]].
