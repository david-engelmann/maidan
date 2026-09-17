# R-04 — SDK claim-shape evidence

## The contract

On `main`, claiming a thread returns a **flattened `ClaimedThread`** object. The mechanism: `crates/maidan-types/src/event_chain.rs:72-76` declares `#[serde(flatten)] pub thread: Thread` plus `pub pin: StrongRef` — so the serialized JSON is `{...thread fields..., "pin": ...}` with no nested `"thread"` key. The repo's own reference example, `examples/lease_demo/lease_demo.py:32`, uses the flattened access pattern (`claim.get("id")`) — this is the ground truth for what the server returns.

## The READMEs

All four SDK READMEs show the nested pattern:

- `sdk/python/README.md` — `res.get("thread")`
- `sdk/typescript/README.md` — nested `thread` access
- `sdk/go/README.md` — nested `thread` access
- `sdk/rust/README.md` — nested `thread` access

## Why this is P0 and not a nit

With the nested pattern, `res.get("thread")` returns `None` (or the equivalent) on a *successful* claim. A builder's claim loop then silently skips every thread. The program runs, exits 0, and does nothing. This is the worst docs failure mode: success-shaped silence. It was rated P0 because it corrupts the first integration attempt, not because it's hard to fix — the fix is a few lines per README.

## How the drift likely happened

The flattened shape appears to be the newer contract (the lease_demo was updated; the READMEs were not). This is the same hand-maintained-duplication mechanism behind the version pins (R-03): the shape is stated in five places, updated in one.

## Test gap

`sdk/python/tests/test_client.py` asserts the claim response is `None`-or-`dict` — an assertion that passes for *both* shapes, which is why the drift survived. A shape-asserting test (concrete key assertions against the `ClaimedThread` contract) would have caught it. Recommended in INIT-03 alongside a possible CI check that extracts claim-shape usage from SDK READMEs and diffs it against the contract source.
