# Cluster 28.0 — Privacy complete (workspace erasure depth)

Cluster 25.0 shipped message-only `POST /workspaces/:id/purge`. Cluster 28 completes
the operator compliance path: related rows removed, audit trail queryable.

> **Goal:** Deep workspace purge (references, tokens, event log) + `GET …/audit`.
>
> **Target tag:** `v28.0.0`.

## PRs

| #          | Title                                                       |
|------------|-------------------------------------------------------------|
| kickoff    | `docs: Cluster 28.0 kickoff` (this doc)                     |
| 28.0.1     | `feat(maidan-store): deep workspace purge`                  |
| 28.0.2     | `feat(maidan-server): audit list + purge metadata`            |
| 28.0.3     | `test: workspace purge postgres + audit e2e`                  |
| 28.0.retro | `docs(retro): Cluster 28.0 + v28.0.0 tag prep`               |

## Exit criteria

- `POST /workspaces/:id/purge` removes messages, dangling references, revokes tokens,
  deletes workspace event log rows; result JSON reports counts.
- `GET /workspaces/:id/audit` returns workspace-scoped audit events.
- Store tests on SQLite + Postgres; server e2e for purge + audit.
- `v28.0.0` tagged after retro.

## Out of scope

- Deleting workspace row, members, channels, artifacts blobs (Track V / future).
- Full Slack GDPR export bundle.

## References

- [[Remaining Work]] §1 (workspace purge gap), [[Retros/Cluster 25.0]].
