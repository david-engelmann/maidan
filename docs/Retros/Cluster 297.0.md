# Cluster 297.0 retro — Rust SDK (0.1.0), SDK arc finale

> Tag **`v297.0.0`**. Phase XXIV (post-gate hardening). SDK arc, part 4 (finale). No new gate tag.

## What shipped

The fourth and final usable language client, to the frozen v1 contract ([[Client Contract]]),
verified black-box through the Cluster-294 harness — **completing the SDK arc (294–297):
TypeScript, Python, Go, Rust, all at 0.1.0.**

- **`sdk/rust/src/lib.rs`** — a `Client` (REST + WebSocket), a **standalone crate that does not
  depend on any `maidan-*` server crate** (the contract's hard constraint). Service handles
  mirror the surface (`client.workspaces()/channels()/threads()/messages()/artifacts()`, plus
  `claim_next_thread` / `renew_claim`). Responses come back as `serde_json::Value` so unknown
  fields are preserved and ignored (forward-compat). `client.mcp_url` is `{base}/mcp/streamable`.
- **`sdk/rust/src/subscribe.rs`** — `subscribe(filter, on_event)` returning a `Subscription`
  (a background reader thread; closes on `close()` or `Drop` by flag + TCP shutdown to unblock
  the parked read), and the `wait_for_{result,mention,ready}` helpers (an mpsc channel +
  `recv_timeout`).
- **`MaidanError`** — `.status` (0 = transport), `.body`, `.retry_after` (429),
  `.is_conflict()` / `.is_forbidden()` / `.is_rate_limited()` / `.is_transport()`; implements
  `std::error::Error`.
- **`sdk/rust/tests/black_box.rs`** — a `cargo test` black-box suite (hero loop; `get_result`
  404; error surfacing; claim-next; WS subscribe), each skipping when `MAIDAN_URL` is unset
  (the repo's Docker-skip convention). **5/5 pass** locally; `cargo clippy -D warnings`,
  `cargo fmt --check`, and the doctest all clean.
- **`Cargo.toml`** bumped 0.0.1 → **0.1.0**; **`README.md`** rewritten; `scripts/sdk-test.sh`
  rust arm + a crate-local `.gitignore` (ignore `target/` + `Cargo.lock`).

Verified locally: **5/5 tests pass**; `cargo build` / `clippy -D warnings` / `fmt --check` clean.

## Surprises / decisions

- **Rust is the one SDK that takes dependencies.** Rust's std has no HTTP or TLS client at
  all (unlike JS `fetch`, Python `urllib`, Go `net/http`), so a zero-dep client would mean
  hand-rolling HTTP/1.1 *and* a TLS stack — impractical and less safe than vetted crates. The
  crate takes a small synchronous stack: `ureq` (REST over rustls; needs the `json` feature for
  `send_json`/`into_json`) + `tungstenite` (WebSocket, `rustls-tls-webpki-roots`) + `serde_json`.
  Documented as the deliberate exception to the SDKs' "stdlib only" story.
- **Standalone crate via an empty `[workspace]` table.** `sdk/rust` sits inside the repo but is
  *not* a workspace member; a bare `[workspace]` in its `Cargo.toml` detaches it (a nested
  non-member crate otherwise errors "believes it's in a workspace when it's not"). This keeps
  its client-only dependency tree entirely out of the repo's strict workspace lint / `cargo
  deny` — and honors "must not depend on `maidan-server`".
- **`serde_json::Value` responses**, not generated typed structs — the same 0.1.0 call as the
  Go client (`map[string]any`) and the TS `.d.ts` (`any`): lean, forward-compatible, no
  server-drift. Typed models are a logged future refinement.
- **`Subscription` closes on `Drop`** (RAII), and `close()` shuts down a cloned `TcpStream` to
  unblock the parked `read()` — Rust's ownership made the "stop a blocking reader thread"
  problem the fiddliest part; the flag + socket-shutdown pattern is robust for the ws:// path
  the harness exercises.
- Same **result-write constraint** as the other three: under the auth-disabled harness the
  acting member is nil (`produced_by` FK), so the test uses `get_result` → 404.

## Capability table extension

New Rust client (`sdk/rust`, 0.1.0), a standalone crate (small sync `ureq`/`tungstenite` stack;
no `maidan-*` dependency). **The SDK arc (294–297) is complete** — four language clients at
0.1.0. No server capability change.

## Risks identified + still open

- **None of the four SDKs are published to their registries** (npm / PyPI / crates.io / `go get`
  by tag). Publishing needs registry tokens as repo secrets + a release trigger on an `sdk-*`
  tag — the maintainer must add the secrets (flagged under the standing publish authorization);
  logged in [[Open Work]].
- **No SDK interop CI yet** — the black-box harness is local-only; a CI job running the scenario
  catalog across the four SDKs (the report-only A2A interop job is the pattern) is a follow-up
  now that all four exist.
- SDK responses are untyped (`Value` / `map[string]any` / `any`) — typed models across the four
  are a future refinement.

## Forward look

The SDK arc is done (four clients, 0.1.0). Natural next steps (all in [[Open Work]]): an SDK
interop CI job, registry publishing on `sdk-*` tags (needs secrets), and typed response models —
plus the broader post-272 backlog (MCP `2026-07-28`, durable mail retry queue, Slack/Git
projectors, public launch).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 296.0]].
Built under the standing SDK-arc authorization (usable 0.1.0, all four languages). The Go
toolchain (296) and Rust verification here were run against a real source-built server via the
harness — every SDK is proven end-to-end, not just compiled.
