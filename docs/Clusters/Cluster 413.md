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
| 413.1 | #1014 | D-B: per-workspace grant ceiling. `maidan_delegation_policies` (Postgres 0107 / SQLite 0106), enforced in the store's grant create, `GET/PUT /workspaces/:wid/delegation-policy` + MCP `get/set_delegation_policy` |
| 413.2 | #1016 | D-A for tokens: mint, attenuate, delegated exchange and revoke commit with their audit row (`*_audited`, `AuditFor<T>`), across REST, MCP, app install, OAuth code exchange (which recorded nothing before), first-admin mint and SCIM deprovisioning (which recorded no per-token revoke). `authority_audit_contract` forbids the unaudited calls in handlers; `authority_audit_tx` proves on both backends that a failed audit write aborts the change |
| 413.3a | #1017 | D-A for delegation grants (create, revoke), share tickets (create, revoke) and the grant ceiling, across REST and MCP. A ticket revoke that finds no live ticket records nothing; a grant revoke stays idempotent and is recorded either way |
| 413.3b | #1019 | D-A for workspace purge, erase and import, message purge, and legal hold place/lift, on REST and MCP; export and secret resolve write their row before releasing anything, and withhold it if they cannot. Three defects found on the way: (1) MCP `import_workspace` restore+force erased a held workspace — REST checked the hold, MCP did not; the hold check now lives in the store, inside the destroying transaction, so no caller can skip it; (2) a forced restore erased and then imported in separate transactions, so a failed import left the workspace gone — now one transaction; (3) audit `actor_id`/`subject_id` were `ON DELETE SET NULL` to members, so erasing a workspace anonymized every audit row its members wrote, the erase's own included — the foreign keys are dropped (pg 0108 / sqlite 0107), as `grant_id` already was. Message purge is now refused under hold too |
| 413.4a | current | D-A for governance and membership, REST and MCP: member freeze/unfreeze, channel membership (including a private channel's creator becoming its admin), review-requirement set/clear, reviewer removal, land-gate clear, governance-skill grants, egress targets, app-installation revoke, secret create/delete — one `GovernanceAuditStore` trait. Fixed on the way: the MCP freeze, land-gate, review-requirement and membership tools recorded nothing; a review requirement was lowered by a non-admin when a concurrent write outran the pre-check (the store now decides inside the write); an app installation's tokens were revoked outside the installation's transaction; MCP `add_reviewer` accepted a member of another workspace |
| 413.4b | planned | D-A for SCIM: provisioning creates the member and its link together; deactivation and deletion revoke every live token in the same transaction and fail — so the IdP retries — instead of reporting success with tokens live |
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
