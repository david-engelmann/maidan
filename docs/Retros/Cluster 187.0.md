# Cluster 187.0 retro — a workspace can leave, not only be deleted

> Tag **`v187.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc B (multi-tenant SaaS operability), part 3.

## What shipped

- `crate::export::build` assembles the workspace content graph (workspace,
  members, channels + members, threads, messages + edits, pins, references) into
  a flat, id-linked JSON bundle, paginating messages per thread to completeness.
- `GET /workspaces/:id/export`, gated on `token:admin`, wired through OpenAPI +
  the capability map. `workspace_export_e2e` proves reader-denied / admin-allowed.

## Surprises

- **DM content came for free.** DM and group-DM threads live in the `__dm__`
  channel, so `list_threads_for_workspace` already includes them — iterating
  threads → messages captures DM message bodies without a separate DM path. (The
  DM *conversation* participant rows are separate; the messages are the content.)
- **The bundle needs no `ToSchema`.** Declaring the OpenAPI response as a
  bodyless `200` (like `purge_workspace`) let the export DTO be `Serialize`-only
  — no fighting `utoipa` derives across ten nested domain types. The bijection
  test only checks route↔capability, not the body schema.

## Decisions

- **`token:admin`, not a new `workspace:export` cap.** The bundle spans private
  channels and DMs, so it must be admin-gated — but a new capability drags in the
  capability-matrix + contract + OpenAPI-security churn. Whoever manages a
  workspace's tokens is effectively its admin, so `token:admin` is a correct,
  bounded gate; splitting a dedicated cap out later is cheap if roles diverge.
- **Flat collections, not deep nesting.** Id-linked lists are easier to diff and
  to re-import than a nested tree, and keep the serializer trivial.
- **Content, not credentials or ops.** Secrets (tokens, webhook/slash/OIDC) and
  operational tables (events, audit, deliveries) are excluded — export is for
  portability of *user data*, and dumping secrets would be a liability.

## Capability table extension

| Change | Where |
|--------|-------|
| `GET /workspaces/:id/export` (token:admin) + `crate::export` assembler | `maidan-server/src/export.rs` + `routes/workspace.rs` |

## Risks identified + still open

- **Net additive, read-only, admin-gated.** Open/tracked in Open Work: no
  reactions/votes (per-message N+1 deferred); no artifact *blobs* (metadata via
  references only); the bundle is built in memory + returned in one response (a
  streaming/NDJSON variant is a follow-up); and there's **no import path yet** —
  export is half of portability; re-import is the other half.

## Forward look

Arc B continues: per-tenant metrics/metering, then a secret-rotation keyring.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
