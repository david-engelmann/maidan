# R-02 — Contracts and auth evidence

## What was read

- `crates/maidan-contracts/` — event kinds source (`event-kinds.json`, 28 kinds), JSON schemas, golden files
- `crates/maidan-auth/` — capability parsing/validation, token attenuation
- The route→capability mapping (reported at the audited commit as `crates/maidan-server/src/capability_map.rs`; at 399.3 enforcement is per-handler — see below)
- `crates/maidan-server/src/routes/` and `src/dm.rs` — enforcement call sites (~260 `cap(&auth, …)` lines, counted at 399.3; the audit's "~43" was an undercount)
- `docs/Claims.md`, `docs/Integration.md`, `docs/Threat-Model.md`
- CI workflows covering the capability matrix, denial matrix, and backend parity

## Strengths (with evidence)

- **Contract discipline is real, not decorative.** 28 event kinds with a single source of truth, 177 MCP tools, golden-file tests, and *bidirectional* consistency tests (contract → code and code → contract). This is the repo's most defensible asset.
- **The capability model is coherent.** Capability-scoped tokens, attenuation (`POST /tokens/attenuate`), and a denial matrix that tests negative cases — most projects test only the happy path.
- **`docs/Claims.md` is unusually honest.** It documents edge cases and failure modes instead of marketing the API. Recommended as the voice reference for future docs (see INIT-10).
- **Fail-closed posture on insecure auth.** The flags that disable auth fail closed rather than silently downgrading — the right default.

## Structural concerns (leading to INIT-07)

- **Enforcement is per-handler, not router-bound.** ~260 `cap(&auth, …)` call sites across ~30 handler modules (counted at 399.3 via the `cap()` helper in `crates/maidan-server/src/routes/mod.rs:74`) each remember to check capabilities. The denial matrix verifies current behavior, but the architecture permits a future handler to forget. No structural fail-closed property.
- **`bypass()` is a single boolean** that disables all checks. Its callers and auditability were not fully traced in this audit — flagged for review rather than asserted as a vulnerability.
- **Capability validation errors are `String`-typed** in `maidan-auth`, limiting programmatic handling.
- **Schema/capability naming gap:** `contracts/ws-subscribe-filter.schema.json` (verified: `$id` is `…/ws-subscribe-filter-v3.json`, description is empty) doesn't reference the gating `event:subscribe` capability, and its v3 isn't tied to `event-kinds.json` versioning.

## What was not fully traced

- Whether all 177 MCP tools inherit the HTTP capability map or carry their own authorization metadata (relevant to INIT-02's open questions).
- The full caller graph of `bypass()`.
- The exact semantics of `MAIDAN_BOOTSTRAP=1` against the capability model (relevant to INIT-05).
