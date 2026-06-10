# Cluster 104.0 — Durable ephemeral state

**Theme:** Persist short-lived but request-path-critical state (OAuth codes, reindex jobs) so it survives pod hops and restarts.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XIX · tag **`v104.0.0`**.

**Predecessor:** [[Clusters/Cluster 101.0]]; app OAuth from [[Clusters/Cluster 57.0]], reindex API from [[Clusters/Cluster 87.0]].

---

## Problem

Two pieces of short-lived state live **only in process memory** and break under multiple replicas (and on restart):

- **App-OAuth authorization codes** — `AppOAuthRuntime.codes` (`crates/maidan-server/src/app_oauth.rs`) is an in-memory `HashMap`. A code minted on replica **A** fails to exchange if the token request lands on replica **B** ("code not found").
- **Reindex job registry** — `crates/maidan-server/src/reindex_ops.rs` tracks job status in memory. `GET /operator/reindex-embeddings/:job_id` on a different replica returns "not found", and any in-progress job's status is lost on restart.

Both sit on real request paths (an OAuth exchange; an operator polling a long job). This cluster persists them to the relational core so any replica can serve them.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Store** | `maidan_oauth_codes` (code hash, app/member, scopes, single-use, `expires_at`) and `maidan_reindex_jobs` (job id, model, state, counts, timestamps) — Postgres **and** SQLite, dialect-parity tests. |
| **Server** | App OAuth code exchange reads/consumes from the store (single-use via conditional `DELETE … RETURNING`); reindex job create/status read from the store. |
| **Tests** | `cross_replica_oauth_and_reindex_e2e`: mint code on A → exchange on B; start reindex on A → poll status on B; job status survives a server restart. |
| **Docs** | [[Production]] note on code TTL + single-use semantics and reindex job persistence; [[Threat-Model]] note that codes are stored hashed, short-TTL, single-use. |

## Non-goals

- Distributed **execution** of reindex jobs — a job still *runs* on the replica that started it; only its **status** is shared/durable. (Cross-replica work scheduling is deferred to [[Clusters/Product Ladder 102+]] Phase XXII / [[Open Work]].)
- A generic durable-job framework — two concrete tables, not an abstraction.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 104.0.1 | `feat(store): oauth_codes + reindex_jobs schema (pg + sqlite)` |
| 104.0.2 | `feat(server): app OAuth code exchange via store (single-use, TTL)` |
| 104.0.3 | `feat(server): reindex job registry via store` |
| 104.0.4 | `test(server): cross_replica_oauth_and_reindex_e2e` |
| 104.0.retro | `docs(retro): Cluster 104.0 + v104.0.0 tag prep` |

## Exit criteria

- An OAuth code minted on any replica is exchangeable exactly once on any replica.
- A reindex job's status is queryable from any replica and survives a server restart.
- Codes are stored hashed with enforced TTL + single-use (no plaintext-code persistence).
- `cross_replica_oauth_and_reindex_e2e` green.
- `v104.0.0` tagged after retro.

## Ordering & risks

- **Independent** of [[Clusters/Cluster 102.0]] / [[Clusters/Cluster 103.0]] — can run in parallel.
- **Risk — single-use race:** two concurrent exchanges of the same code must not both succeed. Enforce atomically with `DELETE … WHERE code_hash = $1 AND expires_at > now() RETURNING …` rather than read-then-delete.
- **Risk — code at rest:** store a hash (not the raw code), short TTL, and purge expired rows (reuse the tombstone/cleanup pattern). Cross-check with [[Threat-Model]].

## References

- [[Clusters/Product Ladder 102+]] Phase XIX
- [[Clusters/Cluster 57.0]] (agent app model), [[Clusters/Cluster 87.0]] (reindex job API)
- [[Production]], [[Threat-Model]], [[Capability Map]], [[Architecture]]
