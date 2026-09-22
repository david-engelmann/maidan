# Cluster 403 retro — budget changes cannot remove caps by omission

> Backfilled close record for Cluster 403 · target tag `v403.0.0`

Cluster 403 shipped in #916 without the mandatory retro, Capabilities,
CHANGELOG, or Roadmap entries. This note restores that close record. It does
not cut the tag: release tags trigger image builds and remain a maintainer
decision.

## What shipped

- **Total replacement is actually total.** REST
  `PUT /threads/:id/budget` and MCP `set_thread_budget` require all four
  dimensions: `max_tokens`, `max_usd_micros`, `max_turns`, and
  `max_wall_secs`. An explicit `null` means uncapped. An omission returns an
  error naming every missing dimension.
- **Partial update is explicit.** REST `PATCH /threads/:id/budget` and MCP
  `update_thread_budget` distinguish absent (leave unchanged) from `null`
  (clear the cap).
- **The merge is atomic.** `BudgetStore::patch_thread_budget` applies the
  patch inside one store transaction on both Postgres and SQLite. Callers do
  not need a read-modify-write sequence that could clobber a concurrent
  change to another dimension.
- **Usage survives both operations.** Replacing or patching maxima preserves
  the accumulated token, USD, turn, and wall usage.
- **Contracts followed the wire.** OpenAPI, REST and MCP capability maps, the
  MCP tool catalog, and the sorted tool-name contract include the new PATCH
  operation.

## What was deferred

| To | What | Why |
|----|------|-----|
| Maintainer | Cut `v403.0.0` | Pushing the tag triggers the release workflow and image builds. |

## Surprises

- `#[serde(flatten)]` on the MCP argument struct silently defeated
  `deny_unknown_fields`. A misspelled `max_wall_seconds` parsed successfully
  until the fields were made explicit. The regression test records the exact
  spelling that exposed it.
- The first budget e2e still sent the old partial PUT body, so the new refusal
  broke it for the right reason. The repaired test checks both a complete
  replace and the error text for a partial one.
- The implementation merged without its close record. That made Open Work
  correctly describe the decision in one section while still naming Cluster
  402 as latest elsewhere.

## Decisions

- **PUT replaces; PATCH merges.** Keeping one ambiguous operation would either
  preserve the silent-cap-removal bug or turn every one-axis change into a
  racy read-modify-write.
- **Clearing is explicit.** `null` widens a budget; omission never does.
- **Totality is checked at the edge.** `Option<T>` collapses missing and null,
  so the request parses as `BudgetPatch` and the handler validates that a PUT
  names every dimension. This produces a useful list of missing fields.
- **Strict arguments stay strict.** The MCP shape names its fields rather than
  flattening `BudgetPatch`, preserving Cluster 398.6's unknown-field guard.

No Architecture ADR is needed: this is wire and concurrency semantics for the
existing Cluster 358 budget envelope. The API-surface table and integration
guide now state those semantics.

## Capability table extension

| Capability | First available in |
|------------|--------------------|
| Total budget replacement that refuses omitted dimensions | `v403.0.0` |
| Atomic partial budget update over REST and MCP | `v403.0.0` |

## Risks identified + mitigated

- **Silent widening:** a caller can no longer remove three caps while changing
  one by omission.
- **Lost concurrent updates:** the partial merge occurs in the store
  transaction on each backend.
- **Typo-shaped widening:** unknown budget fields are rejected on both API
  surfaces.
- **False confidence from unit-only coverage:** pure patch semantics, both
  stores, REST wire behavior, MCP parsing, OpenAPI, and capability contracts
  all have coverage.

## Risks identified + still open

- Wall time is evaluated when usage is reported; it is not an eager reaper for
  a silent worker. Lease expiry remains the recovery mechanism for that case.
- The release tag is still pending maintainer action.

## Forward look

Cluster 403 is complete. Continue from the ranked items in
[[Open Work]]; do not treat this documentation repair as a new implementation
cluster.

## Acknowledgements

#916 implementation and verification; #940 tracks this backfilled close
record.
