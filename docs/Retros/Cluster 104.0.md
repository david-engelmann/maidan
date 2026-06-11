# Cluster 104.0 retro — Durable ephemeral state

> Tag **`v104.0.0`**. Third cluster of Product Ladder 102+ (Phase XIX, scale-out core).

## What shipped

- **OAuth-code store** — `maidan_oauth_codes` (Postgres `0029`, SQLite `0028`) plus `Store::insert_oauth_code` / `consume_oauth_code`. Only the SHA-256 `code_hash` is persisted; single-use + TTL are enforced atomically by `DELETE … WHERE code_hash = ? AND expires_at > ? RETURNING …`. Dialect-parity test. (104.0.1, #286)
- **App OAuth via store** — `app_oauth.rs` drops the in-memory `AppOAuthRuntime` map and the `AppState.app_oauth` field; mint persists `hash_code(code)`, exchange consumes atomically (redirect_uri / PKCE validated after consume, so any attempt burns the code). `app_oauth_e2e` covers happy path, single-use, redirect binding, and PKCE S256. (104.0.2, #287)
- **Reindex jobs via store** — `ReindexJob` / `ReindexJobStatus` move to `maidan-types` (behind the `openapi` feature); `maidan_reindex_jobs` (Postgres `0030`, SQLite `0029`) plus `upsert_reindex_job` / `get_reindex_job` (upsert keyed by `job_id`). The in-memory `ReindexJobRegistry` and `AppState.reindex_jobs` field are removed; start upserts Running, the worker upserts the terminal state, GET reads from the store. (104.0.3, #288)
- **Cross-replica e2e** — `two_replica_durable_state_e2e`: two HTTP servers on one Postgres DB; a code minted on A is exchanged on B, and a reindex job started on A is polled to completion on B. (104.0.4, #289)

## What was deferred

- **Explicit restart-survival assertion** — the exit criteria listed "job status survives a server restart." The cross-replica test proves the stronger multi-process property (two independent servers see the same store-backed state); restart-survival follows from the same persistence and was not separately scripted (it would just re-bind a new server to the same pool). Tracked as a nice-to-have, not a gap in the guarantee.
- **Distributed execution** of reindex jobs — a job still runs on the replica that started it; only status is shared/durable (non-goal, deferred to Phase XXII / [[Open Work]]).
- **Expired-row purge job** — `insert_oauth_code` opportunistically reclaims expired codes on each mint; a periodic sweep for `maidan_reindex_jobs`/idle codes is deferred (low volume, not on a hot path).

## Surprises

- **Latent SQLite apps bug.** 104.0.1's parity test was the *first* test to build an app on SQLite, which surfaced `bad timestamp: premature end of input` in `sqlite/apps.rs::parse_ts` — `maidan_apps.created_at` defaults to `CURRENT_TIMESTAMP` (a naive `YYYY-MM-DD HH:MM:SS` string), but `parse_ts` only accepted RFC3339. Agent-apps tests had been Postgres-only, masking it. Fixed `parse_ts` to accept both. Worth a later sweep for other `CURRENT_TIMESTAMP` columns read through strict RFC3339 parsing.
- **Consume-then-validate is correct.** The OAuth e2e first assumed a `redirect_uri` mismatch preserved the code; it does not — `consume_oauth_code` is an atomic delete that precedes validation, so any exchange attempt burns the code. This matches the legacy in-memory flow (`remove` then validate) and is the stricter, correct single-use behavior; the test now mints a fresh code per scenario.

## Decisions

- **Persist, don't replicate.** OAuth codes and reindex status go to the relational core (the existing store), not a new NOTIFY channel or cache — both are pure store-reads on the request path, so durability + any-replica visibility fall out of one write. Contrast 102/103, which fan *ephemeral* signals over NOTIFY because there is nothing to read back.
- **Hash at rest, atomic single-use.** Codes are stored as SHA-256 hashes, short TTL, single-use enforced by `DELETE … RETURNING` (no read-then-delete race). See `docs/Decisions.md` and [[Threat-Model]].
- **Share the job model.** `ReindexJob` lives in `maidan-types` (the `openapi` feature still gives it `ToSchema`), so store and server share one definition instead of duplicating it across the crate boundary.

## Capability table extension

| Capability | Where |
|------------|-------|
| Durable single-use OAuth codes (any-replica exchange) | `maidan_oauth_codes`, `Store::{insert,consume}_oauth_code`, `app_oauth.rs` |
| Durable reindex job status (any-replica read) | `maidan_reindex_jobs`, `Store::{upsert,get}_reindex_job`, `reindex_ops.rs` |

## Risks

- **Single-use race** — mitigated by the atomic `DELETE … RETURNING`; two concurrent exchanges cannot both return a row.
- **Code at rest** — only the hash is stored, with enforced TTL; expired rows are reclaimed on mint.
- **Reindex worker affinity** — status is durable, but a job whose owning replica dies mid-run stays `Running` until... nothing re-drives it. Acceptable for an operator-triggered, idempotent reindex (re-issue it); flagged for the Phase XXII work-scheduling cluster.

## Next

Cluster **105** — multi-replica scale-out smoke (closes Phase XIX).
