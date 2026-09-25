# Cluster 413 retro — the round-3 decisions

> Post-gate hardening · source record (no `v413.0.0` tag yet; the tag is the maintainer's) · PRs #1014, #1016, #1017, #1019, #1024, #1025 + close record

## Outcome

The maintainer's three round-3 decisions (2026-09-23) are implemented. A
workspace bounds how long a delegation grant may live (D-B). Every change to
who may act, approve, read or receive commits together with its audit row, so
a change that cannot be recorded does not happen (D-A). The two-way
capability intersection stands as the contract (D-C, a decision with no code
change). Working through D-A surface by surface also found, and fixed, defects
that had nothing to do with audit.

| Slice | PR | Result |
|-------|----|--------|
| 413.1 | #1014 | D-B: `maidan_delegation_policies`, a per-workspace grant-lifetime ceiling (default 90 days, 1–3650), enforced in the store; REST and MCP. |
| 413.2 | #1016 | D-A for tokens: mint, attenuate, delegated exchange, revoke — including OAuth code exchange and SCIM per-token revokes, which recorded nothing. `authority_audit_contract`; `authority_audit_tx`. |
| 413.3a | #1017 | D-A for delegation grants, share tickets and the grant ceiling. |
| 413.3b | #1019 | D-A for workspace purge, erase and import, message purge and legal hold; export and secret resolve write before releasing. The legal-hold check moved into the store. |
| 413.4a | #1024 | D-A for governance and membership: freeze, channel membership, review requirements and reviewers, land-gate clear, governance skills, egress targets, app revoke, secrets. |
| 413.4b | #1025 | D-A for SCIM: provisioning is atomic; a deprovision revokes every token or fails. |

## Decisions

- **The audit row goes in the change's own transaction** (D-A, maintainer).
  Routine rows stay best-effort, counted and paged. The store exposes an
  `_audited` form of each authority change, and a contract test forbids handler
  code from calling the unaudited one.
- **Reads that release a whole workspace or a secret write strictly first.**
  An export or a secret resolve that cannot be recorded is withheld.
- **A guard belongs where every caller passes through it.** The legal-hold
  check, and the review-requirement lowering check, moved from handlers into
  the store's transaction, where no surface can skip them and no concurrent
  write can outrun them.
- **An audit row keeps naming its actor.** The audit table's member foreign
  keys (`ON DELETE SET NULL`) were dropped: erasing a workspace had anonymized
  its own history.

## What surprised us

- **The audit work was the way the real defects were found.** Walking each
  authority surface to move its audit row turned up: MCP forced restore
  bypassing the legal hold; a forced restore that could erase and then fail to
  import; the audit table anonymizing an erased workspace's history; message
  purge ignoring a hold; five MCP governance tools recording nothing; a race
  that let a non-admin lower a review requirement; app-installation and token
  revokes outside one transaction; MCP `add_reviewer` accepting a foreign
  member; SCIM deprovisioning reporting success with tokens still live.
- **Mutation testing found what reading did not.** Each fix was checked by
  disabling it and requiring its own test to fail; a bypass subscriber
  widening a notification's audience (#1031) was one such catch.

## Method

Every store change landed on both backends with a transactional test that
installs a trigger making every audit insert fail and asserts nothing changed.
Local gates ran with `--no-fail-fast`, so one flaky suite could not hide the rest.

## Carried forward

- Holding a member's own tombstone under a legal hold (a hold currently keeps
  rows, not the words a member withdraws) is a decision for the maintainer.
