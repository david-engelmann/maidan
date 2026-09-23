# Cluster 413 — the round-3 decisions

> Post-gate hardening · target tag `v413.0.0`

## Contract

David's decisions of 2026-09-23 on the round-3 audit (Open Work,
*Round-3 processing-check dispositions*):

- **D-B:** a workspace bounds how long a delegation grant may live. The
  default is 90 days, a workspace may set 1–3650, and it's set with
  `token:admin` on REST and MCP. The store enforces it where a grant is
  created, so no surface can skip it.
- **D-A:** actions that change authority write their audit row inside the
  change's own transaction, so a failed write aborts the change. Routine
  records stay best-effort, counted and paged.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 413.1 | current | D-B: per-workspace grant ceiling. `maidan_delegation_policies` (Postgres 0107 / SQLite 0106), enforced in the store's grant create, `GET/PUT /workspaces/:wid/delegation-policy` + MCP `get/set_delegation_policy` |
| 413.2 | planned | D-A foundation: `append_audit_in_tx` on both backends, then tokens — mint, revoke, attenuate, delegated exchange, app-token mint, app-installation revoke |
| 413.3 | planned | D-A: grants, share tickets, workspace purge/erase/import, message purge, legal hold; reads (export, secret resolve) write first and release data only on success |
| 413.4 | planned | D-A: governance and membership — governance-skill grants, review-requirement loosening, reviewer removal, land-gate clear, channel membership, member freeze, SCIM users, egress targets |
| 413.close | close record | Retro, ledgers, `v413.0.0` (the maintainer's tag) |

## D-A design

Each authority-changing store method takes its `NewAuditEvent` as a
**required argument** and writes it in the same transaction. This follows the
`*_with_event` pattern of Clusters 205–214. The unaudited forms leave the
`Store` trait, so no caller can make the change without its record. The
guarantee is a type, not a convention. Tests make the audit insert fail with a
trigger and assert the change did not happen.

## Non-goals

- Transactional `mutation` rows. The request-layer fallback runs after the
  handler, so they stay best-effort and paged (D-A option A3, rejected).
- Shortening existing grants when the ceiling is lowered. Revocation does that.
