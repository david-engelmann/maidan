# Cluster 25.0 retro — Privacy & erasure

> Closing wave for Cluster 25.0 · target tag `v25.0.0` (shipped with ladder PR #198).

Cluster 25.0 added workspace-scoped message erasure with audit.

## What shipped

- **PR #198** (`0cffd8f`) — `Store::purge_workspace_messages`, `POST /workspaces/:id/purge`,
  `WorkspacePurgeResult` type, `workspace.purge` audit row, store + HTTP e2e tests.

## What was deferred

| To | What | Why |
|----|------|-----|
| [[Remaining Work]] | Full workspace GDPR erasure | Messages only; embeddings, artifacts, events, members remain. |
| Post-25 | Purge UI + confirmation flow | API-first in 25.0. |
| Track V | Member/channel tombstone cascade | Out of 25.0 scope. |

## Surprises

- Purge uses two SQL passes (tombstone all, then DELETE tombstoned) for consistency with per-message purge.

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| Workspace message purge + audit | `v25.0.0` |

## Risks identified + still open

- Operators may assume purge deletes workspace row — it does not.

## Forward look

Ladder **26–27** in #198. See [[Remaining Work]] §1.

## Acknowledgements

- Maintainer merge #198.
