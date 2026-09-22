# Cluster 405 retro — time-boxed cross-organization incident sharing

> Closing Wave 2 row #26 · released as `v405.0.0`

Cluster 405 turns the H7 "incident room + files" idea into a narrow capability
ticket rather than a new identity, membership, or federation system.

## What shipped

- **#952 — a fail-closed ledger.** `maidan_share_tickets` and its artifact
  allowlist record one workspace, one channel, one owner, an expiry no more
  than 48 hours away, optional revocation, and the SHA-256 digest of a
  once-returned secret. SQLite and Postgres share behavioral tests.
- **#953 — an accountable issuer.** `token:admin` callers create, list, and
  revoke tickets over REST or MCP. Issuance and revocation are audited, while
  response and audit shapes expose neither the credential nor its stored hash.
- **#954 — a separate consumer boundary.** `Authorization: ShareTicket
  maid_share_…` reaches only manifest, paginated thread, paginated message, and
  allowlisted-artifact `GET` routes. Public DTOs omit assignment, claim, lease,
  fencing, and tombstone internals; responses are non-cacheable.
- **Close record — expiration and documentation.** End-to-end coverage proves
  an expired ticket fails exactly like an invalid or revoked one. Architecture,
  threat model, capability, changelog, roadmap, and open-work records now name
  the boundary and its residual risk.

## What was deferred

Nothing required by row #26. Richer external collaboration belongs to a new
contract: adding guests, writes, live subscriptions, search, or multiple
channels would change the trust model rather than extend this ticket quietly.

## Surprises

- A local PostgreSQL test first appeared to be skipped because the sandbox
  could not reach Docker. Running it against the actual service proved the
  backend rather than accepting the skip as evidence.
- An authorization-matrix test sent `{}` to a route whose extractor required a
  body. Axum correctly returned 400 before authorization ran; a valid fixture
  was necessary to test the intended 401/403 boundary.
- Cutting `v404.0.0` while this stack was in flight advanced the changelog but
  not the README image pin. The executable docs-number contract caught the
  mismatch in both coverage and integration CI.
- On macOS, the consumer end-to-end binary spent minutes relinking before a
  sub-second test. Treating silence as a hang would have discarded a valid run;
  bounded observation and the final exit status were the evidence.

## Decisions

- **A share ticket is not an API token.** It has its own authorization scheme,
  parser, route tree, and reduced response types. No API-token context means a
  future capability-map change cannot accidentally widen it.
- **The credential belongs in a header, never a URL.** URLs leak through
  histories, analytics, referrers, and routine access logs. The consumer also
  emits `Referrer-Policy: no-referrer` and cache-prevention headers.
- **Scope is one channel plus exact artifact hashes.** A workspace reference is
  necessary before issuance, and the exact ticket allowlist is checked again
  before reading LocalFS or S3. There is no prefix-shaped grant to widen later.
- **Revocation and expiry are one failure class.** Returning the same 401 body
  as an unknown credential avoids giving an attacker a validity oracle.
- **The public DTO is intentionally lossy.** Assignment and claim machinery is
  operational state for workspace members, not incident evidence for an
  external recipient.

This boundary is recorded in Architecture and Threat Model rather than a
separate ADR: the durable architectural decision is the distinct credential
class and route tree, while the exact wire contract lives in Integration and
OpenAPI.

## Capability table extension

| Capability | First available in |
|------------|--------------------|
| Time-boxed, revocable one-channel share-ticket ledger | `v405.0.0` |
| Audited REST and MCP ticket issuance | `v405.0.0` |
| Dedicated read-only incident-share consumer API | `v405.0.0` |
| Exact-allowlisted LocalFS/S3 artifact consumption | `v405.0.0` |

## Risks identified + mitigated

- **Credential persistence:** only a SHA-256 digest is stored; the raw secret is
  returned exactly once and omitted from list and audit records.
- **Authority confusion:** the share scheme never falls through to bearer or
  session authentication and exposes only four GET routes.
- **Artifact confused deputy:** issuance requires a workspace reference, then
  consumption requires the exact ticket allowlist and a live ticket immediately
  before the object-store read.
- **Private operational-state disclosure:** reduced DTOs omit claim, lease,
  fencing, assignee, and tombstone details.
- **Validity oracle:** invalid, revoked, and expired tickets return the same
  status and body.

## Risks identified + still open

- A stolen live credential can read the deliberately shared channel and files
  until expiry or revocation. The hard 48-hour ceiling bounds, but cannot erase,
  that bearer-token property.
- A reverse proxy can still log the `Authorization` header if an operator
  configures it to do so. Production log hygiene remains an operator duty.

## Forward look

Wave 2 row #26 is closed. Resume from the next ranked open item in
[[Open Work]]; do not grow the share-ticket boundary into guests, writes, or a
federation mesh without a separately pinned contract.

## Acknowledgements

#951 pinned the contract; #952, #953, and #954 delivered the three
implementation slices.
