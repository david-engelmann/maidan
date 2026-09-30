# Threat model

High-level security view of Maidan as it ships on `main`. This is an operator and
integrator document, not a formal audit.

## Assets

| Asset | Location | Sensitivity |
|-------|----------|-------------|
| Workspace data | Postgres / SQLite | Messages, threads, votes, search index |
| Content key-encryption key | Operator secret (`MAIDAN_CONTENT_KEK`), never the DB | Wraps every per-message content key; with a pre-shred backup it recovers withdrawn words |
| Per-message content keys | DB (`maidan_content_keys`, wrapped by the KEK) | Destroyed on tombstone (crypto-shredding) |
| Workspace secrets | DB (`maidan_secrets`, AEAD ciphertext) | Substituted into egress only for allowlisted hosts; never in the event log |
| Delegation grants | DB (`maidan_delegation_grants`) | Let one member act as another, time-boxed and revocable |
| API tokens | DB (`maidan_api_tokens`) | Bearer secrets (hashed at rest) |
| Share-ticket secrets | DB (`maidan_share_tickets`) | SHA-256 hash only; raw `maid_share_…` value returned once |
| App OAuth codes | DB (`maidan_oauth_codes`) | SHA-256 hash only, single-use, short TTL — never the plaintext code |
| Federation peer secrets | DB (encrypted with `FEDERATION_ENCRYPTION_KEY`) | Outbound poll credentials |
| Artifacts | Local FS or S3 | User/agent uploads |
| Audit log | DB | Security-relevant actions |

## Trust boundaries

```text
[Agent / Browser] ------HTTPS+Bearer------> [maidan-server] --SQL--> [Database]
[External recipient] --HTTPS+ShareTicket--^       |            `--> [Artifact store]
                                                   `--> [Peer over A2A HTTPS]
```

- **Untrusted:** MCP clients, HTTP clients, federation peers (authenticate but validate payloads).
- **Trusted:** Operator with DB backup access, host running the server.

## Primary threats

| ID | Threat | Mitigation today | Residual |
|----|--------|------------------|----------|
| T1 | Stolen API token | Capability-scoped tokens; revoke via `DELETE /tokens/:id` | Token usable until revoked |
| T2 | `AUTH_DISABLED` left on in prod / by mistake | **Fail-closed (`v157.0.0`):** `AUTH_DISABLED` is honored only with the explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` acknowledgement and never when `MAIDAN_ENV=production` — either way boot is refused, so a stray flag can't silently open the server | A dev binary with both flags explicitly set is still open by design (intended for seed/test) |
| T3 | Bootstrap routes create admin without auth | `MAIDAN_BOOTSTRAP=1` when auth is on; one workspace via bootstrap; production Docker image built **without** `bootstrap` feature (`v91.0.0`) | Open `/workspaces` if dev binary with `AUTH_DISABLED` or bootstrap left on |
| T4 | Federation peer impersonation | Peer bearer + idempotent ingest | Compromised peer can push events |
| T5 | Artifact exfiltration | Bearer on download; SHA-256 addressing; a per-workspace reference gates every read, so a SHA known from another tenant does not open the blob | A reader in the owning workspace can copy what it can read |
| T6 | SQL injection | `sqlx` parameterized queries | ORM bypass bugs |
| T7 | GDPR right-to-erasure | Tombstoning destroys the message's content key, so the event log, exports and peers keep only ciphertext and the chain still verifies (crypto-shredding); `DELETE /messages/:id/purge` hard-deletes the row; `DELETE /artifacts/:sha` drops a shared artifact's bytes with its last reference (all `token:admin` except the tombstone) | A DB backup taken before the shred plus `MAIDAN_CONTENT_KEK` recovers the words until backup rotation; a peer that ingested the words keeps them unless it ingests the tombstone; legal-hold copies are kept by design |
| T8 | Resource exhaustion / denial-of-service by tenant | Per-client rate limit (`MAIDAN_RATE_LIMIT_MAX`, default 1200 per 60 s) + per-workspace fairness limit (`MAIDAN_WORKSPACE_RATE_LIMIT_MAX`, default 6000 per 60 s); per-connection statement timeout (`v107.0.0`); a global in-flight ceiling (`MAIDAN_MAX_CONCURRENT_REQUESTS`) refuses excess requests with `503` before authentication, and WebSockets have their own ceiling | No hard CPU/IO isolation between tenants on one instance (infra-level) |
| T9 | Tampered or replayed workspace export; credential leak via export | Signed `maidan.workspace.export/1` (Ed25519); verify fail-closed on hash/sig/pin/secret fields; **tokens die on export** (no token/secret continuity) (`v391.0.0`) | Integrity without `MAIDAN_EXPORT_VERIFY_KEYS` is not a trust anchor (anyone who can sign with *a* key can produce a valid file); operator key compromise forges exports |
| T10 | Host rewrites the event log (splice, delete, payload edit) | Per-workspace SHA-256 hash chain on every stored event (`prev_hash` + `content_hash`); `GET /workspaces/:wid/events/verify` 409 fail-closed; federation ingest verifies origin hashes before remap (`v392.0.0`) | A wholly fabricated but internally consistent chain still verifies (hashed, not signed — authorship is T9); the pruned prefix is covered by snapshot catch-up (T11) |
| T11 | Fabricated or gapped event-log history; search index silently diverges | Hashed snapshot + since-LSN catch-up for the pruned prefix (`maidan.event-log.snapshot/1`); CursorTooOld never clamps; search tap verifies backfill and fails loud on `Lagged` without a log (`v393.0.0`) | Snapshot is hashed, not signed — a consistent-but-fabricated graph is T9; `include_graph` is `token:admin` / peer so `workspace:read` cannot dump private channels |
| T12 | Leaked or replayed cross-organization share ticket | Separate `ShareTicket` auth class; header-only credential; one channel; exact artifact-SHA allowlist; hard 48-hour maximum; immediate revocation; dedicated GET-only routes with reduced DTOs; liveness rechecked before an artifact backend read; raw secret returned once and only its SHA-256 digest stored | A stolen live ticket can read its deliberately shared channel and allowlisted files until expiry or revocation; operators must keep authorization headers out of reverse-proxy logs |
| T13 | SSRF through an operator-supplied webhook, slash/FSM hook, federation peer, or A2A push URL | One canonical URL parser rejects credentials, loopback, private, link-local, shared, documentation, multicast, and reserved literals. Every delivery resolves again, rejects the whole answer set if any address is non-public, pins that set into a no-redirect HTTP client, and therefore closes DNS-rebinding and redirect pivots. `MAIDAN_ALLOW_PRIVATE_EGRESS=1` is a development-only loopback escape hatch and is rejected in production. | A public service can itself proxy to a private target; outbound network policy remains useful defense in depth |
| T14 | A delegate acts as another member beyond what it was lent | A delegation grant lives no longer than the workspace's grant ceiling (default 90 days) and is exchanged for a token of at most an hour that *is* the subject, carrying only what the grant, the delegate's own authority and any requested attenuation all allow; one hop only; every use is recorded with the delegate as actor and the subject as subject, refusals included; an approval cannot be borrowed for work the actor did | A grant is trusted until it expires or is revoked |
| T15 | Cross-tenant read or write through an omitted or foreign id (an optional `workspace_id`, a SHA, a subscription filter) | Omitted scope defaults to the caller's workspace, never "all"; `tenant_isolation_e2e` runs every HTTP operation, MCP tool and live stream as a second tenant | A new route that skips the shared extractors is caught only by that suite |
| T16 | Credentials leaking into traces | `Authorization`, `Proxy-Authorization`, `Cookie`, `Mcp-Session-Id`, the Slack and GitHub signatures and `Set-Cookie` are marked `Sensitive` on every span, and the query string (where an OAuth `code` travels) is never recorded | A handler that logs a secret itself |

## Bootstrap hardening options

1. **One-shot seed flag** — `MAIDAN_BOOTSTRAP=1` required for bootstrap routes when auth is enabled (`v1.4.0`); only the first workspace may be created via bootstrap.
2. **IP allowlist** — reverse proxy restricts bootstrap paths to admin CIDR.
3. **Compile-time strip** — production release builds omit bootstrap routes via Cargo feature `bootstrap` (default on for dev/tests; Docker image uses `--no-default-features`) (`v91.0.0`).

Recommended production flow: seed the first admin with `maidan init` (writes through the store — no unauthenticated HTTP routes, no `AUTH_DISABLED`; see [Production.md](Production.md#maidan-init-recommended)), mint per-agent tokens from it, deploy the production image (no bootstrap routes), set `MAIDAN_ENV=production`. The HTTP-bootstrap / `AUTH_DISABLED=1` seed is a private-network-only alternative for dev.

## Related docs

- [OIDC](OIDC.md) — human login (authorization code + PKCE, sessions)
- [Production](Production.md) — env vars and probes
- [Deploy](Deploy.md) — network placement
- `DELETE /messages/:id/purge` — hard-delete after tombstone (Track V.2)
