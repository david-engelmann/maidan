# Cluster 114.0 retro — Coverage uplift + envelope fuzz

> Tag **`v114.0.0`**. Fourth cluster of Phase XXI (correctness & coverage).

## What shipped

- **Envelope round-trip + fuzz tests** for the previously-untested JSON-RPC /
  MCP / A2A wire surface (inline `#[cfg(test)]`, so they count toward
  `--lib --bins` *and* full-suite coverage). (114.0.1)
  - `maidan-mcp/src/protocol.rs` — JSON-RPC request parsing (id / default
    params / explicit-null id → `None`), `success`/`failure`/`parse_error`
    response shapes, notification shape; proptest fuzz over arbitrary
    `(code, message, data)` error envelopes and request method/id.
  - `maidan-mcp/src/error.rs` — every `McpError` variant → its JSON-RPC code,
    message carry, and the `AuthError` / `StoreError` / `serde` / `SearchError`
    `From` conversions.
  - `maidan-a2a/src/protocol.rs` — terminal-state classification, untagged
    `JsonRpcId`, response constructors, `A2aMessage` round-trip + `message_text`
    part-filtering, `Task` camelCase round-trip, `maidan_context_from_metadata`;
    proptest fuzz over `is_terminal_task_state` and `message_text`.
- **Coverage gate now measures the whole test suite.** (114.0.2) The
  `coverage` job previously ran only `--lib --bins` (inline units), which
  capped the number at **~16%** — most store/server logic is exercised by
  integration tests, not units. The job now runs
  `cargo llvm-cov nextest --workspace` with the same `docker:dind` service the
  `integration` job uses, so Postgres testcontainer paths are covered. One
  instrumented run, reported twice (`report --lcov` + `report --html`).
- **`COVERAGE_MIN_LINES` ratcheted `11.0` → `40.0`.** Measured full-suite line
  coverage is **~60%**, so the 40 floor lands with ~20 points of margin.
  Timeout `45` → `75` min for the instrumented full suite.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Cluster 115  | Module split + `unwrap()` purge | Final cluster of Phase XXI. |
| (future)     | Push the floor past 40 toward the ~60% ceiling | 40 is the ladder's stated target; tightening further is a separate ratchet once coverage stabilizes in CI. |
| (future)     | `cargo-fuzz` libFuzzer targets | The "fuzz" here is property-based (`proptest`) round-trip on the envelope surface — sufficient for the exit criterion; a dedicated fuzz harness is heavier and unscoped. |

## Surprises

- **The old gate was nearly meaningless.** `--lib --bins` excludes every
  `tests/` integration suite, so the gated number (16%) bore little relation
  to what the test suite actually exercises (~60%). Raising the *floor*
  without changing the *basis* would have been impossible — no realistic
  amount of inline unit testing moves a 23k-line workspace from 16 → 40 when
  the bulk is DB/HTTP glue tested through integration. The real fix was the
  measurement basis, not more units.
- **`Option<serde_json::Value>` swallows an explicit `null`.** A JSON-RPC
  request with `"id": null` deserializes to `id == None`, identical to an
  absent id — pinned by a test so the behavior is documented, not incidental.

## Decisions

- **Gate on full-suite coverage, with `dind`.** Coverage should reflect code
  actually exercised by the whole suite, not just inline units. This mirrors
  the `integration` job's Docker setup and is the only way the 40 floor is
  both meaningful and attainable. No [[Architecture]] change.
- **Floor at 40, not 60.** The measured ceiling is ~60%, but the gate is set
  to the ladder's target (40) to leave headroom for CI variance (a flaked or
  skipped testcontainer class shouldn't red the gate). Tightening is a later
  ratchet.
- **Property-based round-trip as "fuzz".** Consistent with Cluster 112's
  `proptest` approach; a libFuzzer harness is out of scope.

## Capability table extension

| Capability | Where |
|------------|-------|
| Full-suite coverage gate (≥ 40% lines) | `.github/workflows/ci.yml` (`coverage` job) |
| JSON-RPC / MCP / A2A envelope round-trip + fuzz coverage | `maidan-mcp/src/{protocol,error}.rs`, `maidan-a2a/src/protocol.rs` |

## Risks identified + mitigated

- **Meaningless coverage signal.** The gate now reflects the real exercised
  surface, so a regression that drops coverage below 40% fails CI.

## Risks identified + still open

- **Coverage-job runtime.** The instrumented full suite + testcontainers is
  heavier than the old unit-only run (timeout raised to 75 min). If it proves
  slow/flaky, options are nextest sharding or excluding the slowest e2e
  binaries from the coverage run (not the gate). Coverage is a non-required
  check, so a hiccup doesn't block merges.

## Forward look

Phase **XXI** closes with **Cluster 115 — module split + `unwrap()` purge**:
split `routes.rs` / `tools.rs` by domain and drive non-test
`unwrap()`/`expect()` in `crates/*/src/` to zero with a clippy lint to
enforce it.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
