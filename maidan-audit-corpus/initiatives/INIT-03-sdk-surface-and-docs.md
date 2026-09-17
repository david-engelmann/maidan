# INIT-03 — SDK surface and docs

**Findings:** F-10 (P0), F-11 (P1), F-12 (P2)
**Research:** `research/R-04-sdk-claim-shape-evidence.md`

## Problem statement

1. **Wrong claim shape in all four SDK READMEs (F-10, P0).** The Python, TypeScript, Go, and Rust READMEs show `res.get("thread")` / nested-thread access. Main returns a **flattened `ClaimedThread`** — the `thread: Thread` field is `#[serde(flatten)]`ed (`crates/maidan-types/src/event_chain.rs:72-76`), so the JSON has no nested `thread` key at all. The published snippets silently do nothing on a successful claim: the worst kind of docs bug, because the failure mode is "works, but your code path never runs." (Verified: `sdk/python/README.md:19-20` shows the nested pattern; `examples/lease_demo/lease_demo.py:32` uses the correct `claim.get("id")`.)
2. **No provisioning surface (F-11, P1).** SDKs expose no member-creation or token mint/list/revoke methods. `examples/lease_demo/lease_demo.py` — the repo's own reference example — falls back to a private `_req(...)` call to mint tokens. If the reference example can't stay on the public SDK surface, third-party builders can't either.
3. **Weak SDK test assertion (F-12, P2).** `sdk/python/tests/test_client.py` asserts the claim response is `None`-or-`dict`, which is exactly the assertion shape that lets a schema drift like F-10 pass silently.

## Why it matters to an automation-layer consumer

The SDKs are the primary integration surface for agent frameworks. A builder copy-pasting the README claim loop gets a program that appears to work and never processes a thread. This is a first-15-minutes failure.

## Advisory recommendation

- Fix the four README snippets to the flattened shape (`claim.get("id")` etc.), matching `examples/lease_demo/lease_demo.py`'s actual usage.
- Decide the provisioning question explicitly: either add SDK methods (create member, mint/list/revoke token) or document *why* provisioning stays HTTP-only (a defensible choice — provisioning is a control-plane concern — but it should be stated, not discovered via `_req`).
- Strengthen the SDK claim test to assert the flattened shape's key fields, so the next server-side shape change breaks the SDK test instead of the user's program.
- Consider a CI consistency check that extracts claim-shape usage from SDK READMEs and compares against the `ClaimedThread` contract.

## Open questions for the building agent

- Is the flattened `ClaimedThread` the settled shape, or is a nested shape planned? The README fix should target the shape with a future, not just the shape of today.
- Should provisioning be an SDK concern at all? If the answer is "HTTP-only by design," the lease_demo's `_req` usage becomes the documented pattern rather than an embarrassment — but then document it as such.
- The SDKs are at `0.1.0` with no stability commitment; does fixing the READMEs warrant a `0.2.0` or is `0.x` understood as "examples may drift"?

## Signals of resolution

- All four SDK READMEs show the flattened claim shape; the snippets were executed against `main` (or a contract fixture) as part of the change.
- Provisioning is either in the SDKs or explicitly documented as HTTP-only with a supported example.
- The SDK test asserts concrete claim fields.
