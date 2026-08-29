# Cluster 318.0 retro — token-pack evidence

> Tag **`v318.0.0`**. Phase XXIV (post-gate hardening). Last launch-prep cluster of the
> 2026-08-28 research-sweep program. No new gate tag.

## What shipped

A number for the README's "far fewer tokens" claim, which was assertion-only.

- **`crates/maidan-server/tests/token_pack.rs`** — a measurement (the `load_baseline`
  pattern: `#[ignore]`d harness + pure fns unit-tested in CI). It seeds a channel (8
  threads × 40 substantive messages = 320; the target thread carries 15 edits), then
  compares the serialized bytes of the **scoped context pack** (`build_thread_context`,
  default limits) against the naive baseline of **every message in the channel**, plus the
  same pack with full edit bodies (the lean-edits lever).
- **Measured (in-process SQLite):** scoped pack **~4 951 tokens / 19 802 bytes** vs naive
  channel dump **~33 908 tokens / 135 630 bytes** = **~6.8× fewer tokens**; lean edits vs
  full edit bodies = **~1.3× fewer** on the pack.
- **`Benchmark.md`** gained a "Context-pack token savings" section (method + numbers +
  reproduce command); **`Claims.md`** upgraded the token row from "Shipped" to "Shipped +
  measured" with the ratio + a link to the evidence. No ratio was placed on any page
  without the harness behind it.

## Method / honesty notes

- **Bytes are the headline, exact number** — the serialized JSON is literally what an agent
  receives over the wire. The token count is `≈ chars/4` (a documented rough approximation;
  tiktoken isn't a workspace dep). The **ratio is tokenizer-independent to first order**, so
  the "6.8×" holds regardless of the model's tokenizer.
- The reduction has two levers: the pack is **bounded** (default 100-message window +
  pins/results/refs) while the dump is not — so the ratio grows with channel size — and
  **edits are metadata-only by default** (`include_edits=false`).
- It's a **data-shape** measurement, so it's hardware-independent — unlike the latency/
  throughput numbers, it doesn't depend on the M3 Max run.

## Test evidence

`token_pack` unit tests (the pure `estimate_tokens` / `reduction_ratio`) pass in CI; the
`#[ignore]`d measurement ran locally to produce the numbers; `cargo clippy --tests -D
warnings` clean; mdbook linkcheck green (incl. the Claims.md → Benchmark.md anchor).

## Forward look

**318 closes the launch-prep clusters of the 2026-08-28 sweep (315–318).** Next is the
**fidelity + context flagship arc** — the differentiator: typed reference relations →
glossary → confidence/conventions → as-of context replay → seed-from-message → context
snapshot artifact. Per David's directive, that arc reshapes primitives freely (e.g.,
`Reference.relation` free-string → a controlled type) with no backwards-compat shims. The
public launch stays gated on the maintainer's go; a logged follow-up remains to publish the
quickstart image to GHCR for a true one-command no-clone eval.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Closes the launch-prep leg of the
2026-08-28 sweep ([[Open Work]]).
