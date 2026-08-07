# Cluster 176.0 retro — capability-filtered `tools/list`

> Tag **`v176.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 4 (token round 3), part 2.

## What shipped

- `tools/list` now returns only the tools the caller's token capabilities allow,
  via `tools::catalog_for(auth)` (reuses `required_capability` + `has_capability`).
  Bypass callers still see the full catalog.

## Surprises

- **The auth was already in hand.** `dispatch` already receives `&AuthContext`;
  `tools/list` just wasn't using it. The fix was a new pure filter fn + a
  one-word change in the arm (`catalog()` → `catalog_for(auth)`). Filtering in
  the arm (not `catalog()`) kept the catalog↔contract tests untouched.

## Decisions

- **Filter response, not source.** `catalog()` stays complete for contract tests
  and full-cap callers; only the per-caller *view* is scoped.
- **Descriptions untouched (deferred).** Trimming verbose tool descriptions is a
  separate, fiddlier token lever; not sending whole unusable schemas is the
  bigger, cleaner win.

## Capability table extension

| Change | Where |
|--------|-------|
| Capability-filtered `tools/list` (`catalog_for`) | `maidan-mcp/src/tools/mod.rs` |

## Risks identified + still open

- **Low.** Pure, additive filter; bypass/full-cap behavior unchanged. Worst case
  of a wrong filter is a tool missing from the list — but the caller couldn't
  call it anyway (the dispatch cap-gate is unchanged and authoritative).

## Forward look

Arc 4 continues: lean write-acks / omit-empty metadata; opt-in lean event frames.
Optional: trim catalog descriptions.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
