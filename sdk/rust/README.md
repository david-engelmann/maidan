# maidan (Rust)

Official Rust client for [Maidan](https://github.com/david-engelmann/maidan), the operating
layer for teams of AI agents. **REST + WebSocket** (MCP is a URL, not a dependency; A2A is a
recipe). A standalone crate — it does **not** depend on any `maidan-*` server crate.

The example below is 0.3.0, which is not on crates.io yet: `maidan = "0.1"` returns
`serde_json::Value` and has one error type, so the example does not compile against it.
Until 0.3.0 is published, depend on the repository (Cargo finds the crate in `sdk/rust`):

```toml
[dependencies]
maidan = { git = "https://github.com/david-engelmann/maidan" }
serde_json = "1"
```

```rust
use maidan::{Client, MaidanError};
use serde_json::json;

fn main() -> Result<(), MaidanError> {
    let client = Client::new("http://127.0.0.1:8080", ""); // or Client::from_env()

    // Hero loop: claim the next ready task, do work, post, set a result.
    // A claim returns the thread's fields at the top level (plus a
    // content-addressed `pin`), or None when nothing is ready.
    if let Some(claim) = client.claim_next_thread(channel_id, None)? {
        client.messages().post(&claim.id, "on it")?;
        client.threads().set_result(&claim.id, json!({ "ok": true }))?;
        // Long job? Heartbeat the lease with the fencing token the claim returned.
        if let Some(lease) = &claim.claim_lease_id {
            client.renew_claim(&claim.id, lease, 300)?;
        }
    }

    // Errors are variants, one per problem type.
    match client.threads().get(thread_id) {
        Err(MaidanError::NotFound(problem)) => println!("gone: {:?}", problem.detail),
        other => drop(other?),
    }

    // React to work instead of polling.
    let sub = client.subscribe(
        json!({ "workspace_id": wid, "kinds": ["message_posted"] }),
        |e| println!("event {} {}", e["kind"], e["thread_id"]),
    )?;
    // sub.close(); // (also closes on drop)

    // Or block until a specific signal (wraps subscribe):
    let _ready = client.wait_for_ready(wid, None, std::time::Duration::from_secs(30))?;
    Ok(())
}
```

- Constructor: `Client::new(base_url, token)` or `Client::from_env()` (`MAIDAN_URL` /
  `MAIDAN_TOKEN`). `client.mcp_url` is `{base_url}/mcp/streamable`.
- Errors are a `MaidanError` enum with a variant per RFC 9457 problem `type` the server
  documents (`NotFound`, `Conflict`, `Forbidden`, `CursorTooOld`, `Overloaded`, …; see
  `PROBLEM_TYPES`), `Unknown` for a type this crate does not know or a body that is not a
  problem, `Transport` when there was no HTTP answer and `Decode` when a 2xx body did not fit
  its model. Each HTTP variant holds a `Problem` (`status`, `problem_type`, `title`, `detail`,
  `raw` as sent, `retry_after` on 429 and 503, `snapshot()`); `.status()`, `.problem()`,
  `.is_conflict()` / `.is_cursor_too_old()` / `.is_forbidden()` / `.is_rate_limited()` /
  `.is_transport()` work on any error.
- **0.3.0 (unreleased; 0.2.0 was never tagged):** writes send an `Idempotency-Key` reused across retries; requests retry up to `.with_max_retries(n)` (default 2) on transport failures, 408, 429 (`Retry-After`), 500, 502, 503, 504 and 409 `idempotency-key-in-flight`. `threads().list_all(cid, n)` and `list_events_all(wid, q)` are iterators over every page, asking for at most `MAX_PAGE_SIZE` (500, the server's cap) per page. Typed responses and the error enum are new since 0.1.
- Responses are serde structs in `maidan::models` (re-exported at the root: `Thread`,
  `ClaimedThread`, `Message`, `ThreadContext`, `StoredEvent`, …), from the server's OpenAPI
  schemas and checked against a live server by `tests/black_box.rs`. Members the server adds
  later land in each model's `extra` map (`unknown_members()` lists them); string enums
  (`ThreadState`, …) have an `Other(String)` variant for values this crate does not know.
  JSON the producer chose (`ThreadResult::result`, `StoredEvent::payload`) stays
  `serde_json::Value`, and so do event frames from `subscribe`, whose shape follows `kind`.
- `threads().transition(id, action)` takes the action string; `claim_next_thread(cid,
  lease_secs)` takes an optional lease length.
- Surface (frozen v1): `workspaces().{create,get,import}`, `channels().{list,create}`,
  `threads().{create,get,context,transition,set_result,get_result}`, `claim_next_thread`,
  `renew_claim`, `messages().{list,post}`, `artifacts().{upload,get,meta}`, `subscribe`,
  `list_events`, `follow` (HTTP backfill then WS), and the `wait_for_*` helpers. See the
  repo's `docs/Client Contract.md`.
- Caching (0.3.0): `client.channels().boot(cid)` returns the channel's boot prefix as served, with its sha256 (for `evidence.pack_sha256`). `cached_prefix(provider, text, ttl)` places it with a cache breakpoint, `cache_key(workspace_id, group)` and `cache_key_fields(provider, key)` give one cache key per shared-prefix group, never shared across workspaces, and `gateway_session(gateway, thread_id, path, name)` passes the thread id as an OpenRouter, Helicone, LiteLLM or TensorZero session id. The hash takes `sha2`, the crate's one new dependency. See the repo's `docs/Harness Caching.md` for where each harness puts Maidan's bytes.

Rust's standard library has no HTTP or TLS client, so this crate takes a small synchronous
stack (`ureq` over rustls for REST, `tungstenite` for the WebSocket) — the one place the four
Maidan SDKs diverge from "stdlib only". Versioned independently of the server; `0.1.0` is the
first usable release.
