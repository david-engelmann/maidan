> **Reconciled (Cluster 291, 2026-08-27):** David gave the go — the actionable items from
> this pack are folded into [Open Work](Open%20Work.md), the single canonical backlog
> ("Adoption & ecosystem" section). The "new-files-only / do not fold / do not splice into
> Open Work" rules below are **superseded**; this doc now serves as the detailed spec/index
> behind those backlog items. The `sdk/` scaffolds remain gated 0.0.1 name-holds — "do not
> implement the client code without a go" still stands.
# Client Testing — black-box scenarios that cover the server

**Audience:** whoever implements SDK 0.1 and whoever owns CI.
This file is the test plan for the clients **and** a way to hit
the running server from the outside.

**Companions:** [Clients.md](Clients.md), [Client Contract.md](Client%20Contract.md).

**New-files-only.** Do not splice this into Open Work, Handoff,
README, `.github/workflows/ci.yml`, or book/SUMMARY until David
says to. The CI job shape below is what to add when implementation
starts; do not add the workflow in this pass.

Snapshot: 2026-08-26. Server `main` is **v280.0.0**. Existing CI
already has `lint`, `coverage` (llvm-cov, `COVERAGE_MIN_LINES=40`),
`e2e` (compose.quickstart + federation), `scale-out-smoke`.

---

## 0. Why this is also server coverage

Rust unit tests and `maidan-server` e2e already cover store
invariants, FSM, and in-process replicas. They do **not** cover:

- A stranger's HTTP client hitting the **published** routes
- Capability 403s as a Bearer token actually presents them
- WS subscribe / resume / unknown-`kind` as a real socket
- MCP Streamable HTTP as a 2024 host actually speaks it
- A2A JSON-RPC as a peer actually POSTs it
- compose.quickstart as the image a user would run (Postgres +
  MinIO + the Dockerfile), not an in-process test harness

Client tests are black-box against a server built from **this
commit**. A regression in `claim-next`, the capability gate, the
WS filter schema, or an MCP tool name will fail here even if
llvm-cov still says 40%. That is the point. Do not rewrite store
tests in Python.

llvm-cov line % will barely move (the clients are not Rust server
code). Treat these as **route / transport coverage**, reported as
scenario pass/fail, not as a second coverage number.

---

## 1. Fixture

Reuse the stack the `e2e` job already builds:

1. `docker build` `maidan-server:dev` from
   `crates/maidan-server/Dockerfile` with
   `MAIDAN_ENABLE_BOOTSTRAP=1` (same flags as `.github/workflows/ci.yml`
   job `e2e`).
2. `docker compose --profile full up`.
3. Wait for `GET /health` → `{ "status": "ok" }`.
4. Auth: AUTH_DISABLED on quickstart is fine for the happy path.
   Scenario C needs a **fixture token** missing
   `thread:transition` / `message:post` / `event:subscribe`. If bootstrap cannot
   mint that, skip C with a named skip, do not silent-pass.
5. Seed: one workspace, one channel, one member. Prefer the
   existing `scripts/quickstart-two-agents.sh` helpers over a
   third seeder.

Env the SDKs already document: `MAIDAN_URL=http://127.0.0.1:8080`,
optional `MAIDAN_TOKEN`.

Tear down with `docker compose --profile full down -v` like `e2e`.

Do **not** stand up a second compose file for clients. Do **not**
clone the repo again. Do **not** hit a hosted instance from CI.

---

## 2. Shared scenario catalog

Every language runs the same IDs. Names are stable so a failure
in TS and a failure in Python are comparable. Implement as
pytest / node:test / `#[tokio::test]` that call the **SDK**, not
raw httpx (except M and A, which are door canaries).

### H — Hero loop (the 278 loop)

Must pass in Python, TypeScript, and Rust.

1. `claim_next_thread` on an empty channel → empty / 204 / 404
   (confirm the live status against OpenAPI; one of those, not
   "whatever").
2. Create a ready task thread (REST seed is fine).
3. `claim_next_thread` returns it. Lease is held.
4. `threads.context` / `messages.list` see the seed.
5. `messages.post` a reply.
6. `threads.set_result`.
7. A second client `wait_for_result` (WS) unblocks on
   `thread_result_set`.
8. `renew_claim` from the holder succeeds; from a third party
   fails.

This is the README snippet. If H fails, 0.1 does not ship.

### C — Capabilities

Hits the HTTP capability map, not the SDK's error wrapping only.

1. Token **without** `thread:transition` → `claim_next_thread`
   is **403**, not 401, not empty.
2. Token **without** `message:post` → `messages.post` is 403.
3. Token **without** `event:subscribe` → `subscribe` is 403.
4. `workspaces.import` with a non-admin token is 403.
5. There is **no** `GET /workspaces` list. The SDK must not
   expose `workspaces.list`. A raw `GET /workspaces` is 404
   (or whatever OpenAPI says — not 200 with an array).

### Q — Queue / claim semantics (server bugs this will catch)

1. Unready thread is skipped by `claim_next_thread`.
2. Skill mismatch is skipped (if the fixture can set skills;
   otherwise skip this row with a name, do not fake it).
3. Second `claim_next_thread` while leased does not hand the
   same thread to a sibling.
4. After lease expiry (or explicit transition back), it is
   claimable again.

These are the claim-next bugs unit tests have already had to
chase. Hitting them over HTTP is the extra coverage.

### W — WebSocket

1. Subscribe with `workspace_id` → `subscribe_ack` plus
   `resume_token` / `after_id`.
2. `messages.post` from another client → `message_posted`.
3. Unknown `kind` in `kinds[]` (or an unknown inbound kind) is
   ignored, not a disconnect.
4. Resume with `resume_token` does not replay already-acked
   events (at-least-once is fine; silent loss is not).
5. Wait helpers time out with a typed error, not a hang. Cap at
   a few seconds in CI.

### R — Retries and errors

1. 429 with `Retry-After` is retried by the SDK; the test can
   stub this at the HTTP layer if the live server cannot be
   pushed into 429 cheaply. Prefer live if a test-only knob
   exists; do not add a server knob just for this.
2. 409 is a distinct error class.
3. 5xx is not retried forever. Bound it.

### I — Idempotency / types

1. Passing a thread id as a channel id is a type error in
   Python (runtime check if not typed), TypeScript (compile +
   a runtime test), Rust (does not compile; a doctest is
   enough).
2. Extra JSON fields on a thread body are ignored.

### M — MCP canary (Python only, not in the SDK package)

Uses `mcp` pinned like `examples/langchain_maidan.py`
(`mcp>=1.9,<2`). Speaks Streamable HTTP at
`POST /mcp/streamable`. Protocol **`2024-11-05`** until J3.

1. List tools includes `claim_next_thread`, `post_message`,
   `wait_for_result`.
2. After REST seed, MCP `claim_next_thread` returns the ready
   thread.
3. MCP `post_message` + `wait_for_result` complete the hero
   loop.

This covers the MCP router the SDK does not touch. Keep
`examples/langchain_maidan.py` and `autogen_maidan.py` as
smoke that CI runs on a schedule or on this job — they are
the framework door.

When J3 lands, this canary must negotiate `2026-07-28` or
fail loud. Do not leave it passing on 2024 after J3.

### A — A2A smoke (one language, Python is fine)

1. `GET /.well-known/agent-card.json` returns JSON with
   `rpc_url`. If a strict A2A v1.0 reader rejects the card,
   **skip with a reason** ("J4: custom card"), do not fail 0.1
   and do not silent-pass as "A2A works."
2. `POST /a2a/v1/rpc` `SendMessage` with a text part returns a
   task or an honest error.
3. Egress parts are text. Do not assert file parts (J5).

### N — n8n-shaped webhook (optional in 0.1, required before
public "works with Zapier" copy)

1. REST-create a webhook on the workspace.
2. `messages.post` (or a mention) produces a signed POST the
   test server records.
3. Bad signature is rejected.

Ship N when J7 copy ships. Not a blocker for SDK 0.1.

---

## 3. What this hits in the main app

| Scenario | Server surface | Why unit tests are not enough |
|----------|----------------|-------------------------------|
| H, Q | `POST /channels/{cid}/threads/claim-next`, lease, readiness | Real HTTP + Postgres + the Dockerfile entrypoint |
| C | `contracts/http-capability-map.json` gates | Bearer as an external client sends it |
| W | `GET /ws/subscribe`, event kinds, resume | Real socket, not a mocked notifier |
| M | MCP Streamable HTTP, `mcp-tool-names.json` | 2024 session model as a host speaks it |
| A | `/a2a/v1/rpc`, Agent Card | Peer-shaped JSON-RPC |
| H seed | `POST /workspaces`, channels, threads | Confirms there is still no MCP create-* |

If a scenario needs a new test-only HTTP knob, stop. Use bootstrap
or AUTH_DISABLED. Do not grow a debug API for the SDK.

---

## 4. CI shape (when implementation starts)

Add a job `clients` to `.github/workflows/ci.yml` (or a
`clients.yml` workflow that `workflow_run`s after `e2e` if
that is cheaper). Do not fork the compose stack.

Suggested steps:

1. Same `docker build` + `compose --profile full up` + health
   wait as job `e2e`. Sharing an image cache with `e2e` is
   the win. A combined job is fine if the timeout stays honest.
2. `pip install -e sdk/python` + pytest `sdk/python/tests`
   (H C Q W R I M; A skip-or-pass).
3. TypeScript: install from `sdk/typescript`, run tests.
4. Rust: `cargo test -p maidan --manifest-path sdk/rust/Cargo.toml`
   (the **client** crate). Never `cargo test -p maidan-server`
   from this job except the health already done.
5. Run `examples/langchain_maidan.py` / `autogen_maidan.py`
   against the same URL if their extra deps are cheap; otherwise
   nightly.
6. Compose down `-v` in `always()`.

Path filters (if a separate workflow): run on changes to
`sdk/**`, `contracts/**`, `examples/**`, `compose.quickstart.yaml`,
`crates/maidan-server/**`, `crates/maidan-mcp/**`, and any crate
that owns claim-next / events / a2a. A server-only PR that
touches claim-next **must** run this job. That is the coverage
bargain.

Release is **independent of server tags.** `v281.0.0` does not
publish PyPI / the JS registry / crates.io. Publish only from an
explicit `sdk-*` tag, and only if this job is green. 0.1.0 does
not publish if `clients` is red. A stale client is worse than none.

Nightly: already have `nightly.yml`. Add M + the two framework
examples there if they are too slow for every PR.

---

## 5. Local loop

```
docker compose --profile full up -d
# wait for /health
cd sdk/python && pytest
# then sdk/typescript, then cargo test in sdk/rust
```

`scripts/quickstart-two-agents.sh` remains the human demo. Do
not replace it with pytest. Do make sure H is a stricter
version of what that script does.

---

## 6. Do not

- Do not duplicate `maidan-store` / FSM unit tests in Python.
- Do not hit production or maidan.world from CI.
- Do not generate tests from the full OpenAPI.
- Do not skip H. Skip C / A / N only with a named reason.
- Do not add MCP as a dependency of the published Python
  package just to run M. M lives in `sdk/python/tests` or
  `examples/` extras.
- Do not call this job "coverage" or feed it to llvm-cov.
- Do not implement from this file without a go. Do not add the
  workflow in the docs-only pass.