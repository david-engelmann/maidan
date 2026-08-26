# Cluster 280.0 retro — framework integration recipes

> Tag **`v280.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1:
> LangChain / AutoGen / REST recipes.** No new gate tag.

## What shipped

Copy-paste, live-verified recipes so an integrator can point an agent framework at
Maidan in minutes, plus the one catalog fix that unblocked a real framework:

- **`examples/`** — runnable clients, each ~a screenful:
  - `langchain_maidan.py` — `MultiServerMCPClient` over `streamable_http` →
    `await client.get_tools()` loads all **78** Maidan tools as LangChain tools.
  - `autogen_maidan.py` — `StreamableHttpServerParams` → `mcp_server_tools(params)`
    loads them as AutoGen tools.
  - `rest_maidan.py` — a framework-independent `httpx` client (thread context + post),
    the most stable surface.
  - `README.md` — indexes the three with install lines and the version pins.
- **`docs/Framework Integrations.md`** — prose recipes + the endpoint/token contract
  (each agent gets its own capability-scoped token), the load-bearing `mcp>=1.9,<2`
  pin, and AutoGen's every-parameter-needs-a-`type` requirement. Wired into the
  published book (`SUMMARY.md` + `sync-docs.sh` copy set, hyphenated path) and linked
  from the README Documentation table.
- **`crates/maidan-mcp/src/tools/catalog.rs`** — `set_thread_result`'s `result`
  parameter now declares `"type": "object"`. AutoGen converts each tool's input schema
  to a strict Pydantic model and rejected the one untyped parameter; every catalogued
  parameter now has a `type`.

## Surprises / decisions

- **Verified live, didn't ship snippets on faith.** Both recipes were run against a
  live Maidan (the quickstart). That is how the two real gotchas surfaced:
  1. **`pip install langchain-mcp-adapters` alone pulls `mcp` 2.x**, whose stateless
     `2026-07-28` rewrite removed modules the current LangChain/AutoGen adapters import
     (`ModuleNotFoundError: No module named 'mcp.shared.session'`). Pinning
     `"mcp>=1.9,<2"` (resolved 1.29.x) fixes both adapters. Documented as a warning
     callout, not buried.
  2. **AutoGen's `autogen_core` schema→Pydantic converter requires every tool
     parameter to declare a `type`** (`UnsupportedKeywordError: ... missing type`). Our
     catalog had exactly one untyped param (`set_thread_result.result`); fixed at the
     source so the invariant now holds catalog-wide.
- **The catalog fix is source-level, not a doc workaround.** Rather than tell users to
  avoid one tool, the schema was corrected so AutoGen loads the whole catalog.
- **Interop CI is honestly deferred.** The recipes are pinned and verified, but a
  required "run-the-adapters" CI job (init → list tools → one read → one write →
  denied-channel check) is a network-touching, adapter-version-fragile job; it stays a
  logged follow-up rather than a flaky gate. The doc's "Keeping these honest" section
  tells a bumper to re-run each example before moving the pins.
- **Reference stays generated, not committed.** Regenerating `book/src/mcp-reference.md`
  from the changed catalog produced a 1255-line diff (the committed copy is stale;
  `docs.yml` regenerates it before every build). Per the repo convention the regen was
  reverted — `catalog.rs` is the source of truth, CI publishes the fresh reference.

## Capability table extension

| Change | Where |
|--------|-------|
| LangChain / AutoGen / REST recipes (runnable, live-verified) | `examples/` |
| Framework-integrations guide + `mcp<2` pin + AutoGen type rule | `docs/Framework Integrations.md`, `book/src/SUMMARY.md`, `book/sync-docs.sh` |
| Every catalog tool parameter now declares a JSON-Schema `type` | `crates/maidan-mcp/src/tools/catalog.rs` |

## Risks identified + still open

- **Adapter/pin drift.** MCP adapters move fast; the pins are known-good today. The
  "Keeping these honest" section prescribes re-verifying before a pin bump. The
  automated interop CI job that would catch drift stays a logged follow-up.
- **`mcp` 2.x compatibility is the adapters', not ours.** Maidan speaks MCP
  `2024-11-05`; when the frameworks support the 2.x SDK the pin loosens. Tracked with
  the "MCP `2026-07-28` upgrade" item.

## Forward look

Remaining launch-readiness backlog (approved order): a **published benchmark**
(reproducible post→observer latency + sustained msgs/sec off the Cluster 198 loadgen),
then **A2A v1.0 compliance** (the largest arc, seeded by the review's gap matrix), plus
the deferred **framework interop CI** job.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 279.0]]. Recipes seeded by the external launch-readiness review
(Cluster 274), then rebuilt and verified live here.
