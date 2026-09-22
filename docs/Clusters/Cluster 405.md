# Cluster 405 — Wave 2 #26: time-boxed cross-organization incident sharing

> Post-gate hardening · target tag `v405.0.0` · umbrella issue #951

## Contract

- A share ticket belongs to exactly one workspace and one channel, names one
  accountable internal owner, and carries an explicit allowlist of artifact
  SHA-256 values already linked to that workspace.
- Its lifetime is at most 48 hours and it can be revoked immediately. The raw
  `maid_share_…` credential is returned once; only its SHA-256 digest persists.
- Issuance is a `token:admin` operation over REST and MCP and is audited without
  the credential or its digest.
- Consumption uses `Authorization: ShareTicket <secret>` on a dedicated,
  read-only `/share` route tree. It never creates an API-token context.
- The public view is limited to a manifest, channel threads, thread messages,
  and exact-allowlisted artifact bytes. Cross-channel and non-allowlisted
  resources are indistinguishable from missing ones.
- Invalid, expired, and revoked tickets fail identically. Ticket liveness is
  checked again immediately before an artifact backend read.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 405.1 | #952 | Types, migrations, and dual-backend ticket/allowlist store |
| 405.2 | #953 | Audited REST and MCP issuer lifecycle plus OpenAPI capability contracts |
| 405.3 | #954 | Dedicated `ShareTicket` consumer boundary and end-to-end contract tests |
| 405.close | close PR | Expiration proof, threat model, ledgers, and retrospective |

## Non-goals

- A guest member, external workspace membership, or act-as-any authority.
- Writes, reactions, claims, assignment, search, events, or live subscriptions.
- A federation mesh, multi-home identity, or a ticket spanning multiple
  workspaces or channels.
- Prefix, folder, or bucket grants. Artifact access is an exact SHA allowlist.

## Definition of done

- SQLite and Postgres enforce the same ledger invariants.
- REST and MCP issuer surfaces are twins and never re-expose the secret.
- The consumer route tree accepts no ordinary bearer or session authority.
- Revoked, expired, and invalid credentials have the same observable failure.
- OpenAPI, capability maps, integration docs, threat model, changelog, roadmap,
  open-work ledger, and cluster retro agree.
