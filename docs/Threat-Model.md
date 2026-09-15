# Threat model (Track V.1)

High-level security view for Maidan `v1.1.0`. This is an operator and
integrator document, not a formal audit.

## Assets

| Asset | Location | Sensitivity |
|-------|----------|-------------|
| Workspace data | Postgres / SQLite | Messages, threads, votes, search index |
| API tokens | DB (`maidan_api_tokens`) | Bearer secrets (hashed at rest) |
| App OAuth codes | DB (`maidan_oauth_codes`) | SHA-256 hash only, single-use, short TTL — never the plaintext code |
| Federation peer secrets | DB (encrypted with `FEDERATION_ENCRYPTION_KEY`) | Outbound poll credentials |
| Artifacts | Local FS or S3 | User/agent uploads |
| Audit log | DB | Security-relevant actions |

## Trust boundaries

```text
[Agent / Browser] --HTTPS+Bearer--> [maidan-server] --SQL--> [Database]
                         |                              `--> [Artifact store]
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
| T5 | Artifact exfiltration | Bearer on download; SHA-256 addressing | Guessable SHA if leaked elsewhere |
| T6 | SQL injection | `sqlx` parameterized queries | ORM bypass bugs |
| T7 | GDPR right-to-erasure | Tombstone then `DELETE /messages/:id/purge` (`workspace:write`) | DB backups may retain bytes until backup rotation |
| T8 | Resource exhaustion / denial-of-service by tenant | Per-client rate limit (`MAIDAN_RATE_LIMIT_MAX`) + per-workspace fairness limit (`MAIDAN_WORKSPACE_RATE_LIMIT_MAX`, `v110.0.0`); per-connection statement timeout (`v107.0.0`) | No hard CPU/IO isolation between tenants on one instance (infra-level) |
| T9 | Tampered or replayed workspace export; credential leak via export | Signed `maidan.workspace.export/1` (Ed25519); verify fail-closed on hash/sig/pin/secret fields; **tokens die on export** (no token/secret continuity) (`v391.0.0`) | Integrity without `MAIDAN_EXPORT_VERIFY_KEYS` is not a trust anchor (anyone who can sign with *a* key can produce a valid file); operator key compromise forges exports |
| T10 | Host rewrites the event log (splice, delete, payload edit) | Per-workspace SHA-256 hash chain on every stored event (`prev_hash` + `content_hash`); `GET /workspaces/:wid/events/verify` 409 fail-closed; federation ingest verifies origin hashes before remap (`v392.0.0`) | A wholly fabricated but internally consistent chain still verifies (hashed, not signed — authorship is T9); after retention prune the dropped prefix is unverifiable until snapshot catch-up (#33) |

## Bootstrap hardening options

1. **One-shot seed flag** — `MAIDAN_BOOTSTRAP=1` required for bootstrap routes when auth is enabled (`v1.4.0`); only the first workspace may be created via bootstrap.
2. **IP allowlist** — reverse proxy restricts bootstrap paths to admin CIDR.
3. **Compile-time strip** — production release builds omit bootstrap routes via Cargo feature `bootstrap` (default on for dev/tests; Docker image uses `--no-default-features`) (`v91.0.0`).

Recommended production flow: seed the first admin with `maidan init` (writes through the store — no unauthenticated HTTP routes, no `AUTH_DISABLED`; see [Production.md](Production.md#maidan-init-recommended)), mint per-agent tokens from it, deploy the production image (no bootstrap routes), set `MAIDAN_ENV=production`. The HTTP-bootstrap / `AUTH_DISABLED=1` seed is a private-network-only alternative for dev.

## Related docs

- [[OIDC]] — planned human login (v2.0.0); design spike in v1.4.2
- [[Production]] — env vars and probes
- [[Deploy]] — network placement
- `DELETE /messages/:id/purge` — hard-delete after tombstone (Track V.2)
