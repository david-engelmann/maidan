# Cluster 317.0 retro — Bet 2 MCP snippet pack + the two-language lease demo

> Tag **`v317.0.0`**. Phase XXIV (post-gate hardening). Third cluster of the 2026-08-28
> research-sweep program. No new gate tag.

## What shipped

The MCP "hero pack" — copy-paste connection + a filter to the six tools that matter — and
the **falsifiable hello-world** the whole positioning rests on: two agents in two languages
sharing one lease board.

- **Two-language lease demo** (`examples/lease_demo/` + `scripts/lease-demo.sh`) — a Python
  SDK worker and a TypeScript SDK worker both call `claim_next_thread` on the same channel;
  Maidan hands each open task to **exactly one** worker (no double-claim across languages),
  and the drained queue returns `null`. **No LLM.** Verified end-to-end locally: Python
  claimed task-1, TypeScript claimed task-2 (distinct), the third claim was `None`.
- **Hero-6 filter** — rewrote `langchain_maidan.py` + `autogen_maidan.py` to load the catalog
  and **filter to** `claim_next_thread` / `post_message` / `get_thread_context` /
  `set_thread_result` / `wait_for_result` / `wait_for_ready` before handing tools to the
  agent, instead of dumping all ~78. **Filter only — the catalog is unchanged server-side**
  and the pi 8-method seam stays callable.
- **MCP client configs** — `examples/cursor-mcp.json` + `examples/claude-desktop-mcp.json`
  (point at `/mcp/streamable`, bearer, `2026-07-28` stateless — no session id).
- **Docs** — `examples/README.md` reworked around the hero demo + MCP configs + auth-on
  quickstart (was "auth disabled, omit the token" — stale since 313); `Framework
  Integrations.md` leads with "don't hand an agent 78 tools — filter to the hero-6"; fixed the
  last stale `rest_maidan.py` auth note.
- **CI** — the validate-quickstart step now guards the new scripts/configs (`bash -n`,
  `node --check`, `py_compile`, JSON-parse). The full two-language run is verified locally
  (`scripts/lease-demo.sh`), like the quickstart demo.

## Surprises / decisions

- **The SDKs already had the whole lease loop** (`claim_next_thread`/`set_result`/
  `wait_for_result`/`wait_for_ready` in both Python and TS, over the frozen v1 contract) — no
  SDK changes needed. But the **Python SDK has no `members.create`** (member bootstrap isn't a
  first-class SDK op), so the demo seeds its two members via the SDK's raw `_req`. Honest: an
  orchestrator normally seeds members out of band.
- **`claim_next_thread` returns `Option<Thread>`** (a Thread or `null`) — the workers key on
  that: a non-null claim = you own the task, `null` = the queue is drained/leased.
- **"Filter, not amputate"** held as a hard line (§0.4 of the sweep's strategy doc): the hero
  pack is a *client-side* filter on `get_tools()`; MCP `tools/list` still returns the full
  (capability-filtered) 78. No `seed_thread_from_message` was added.
- Kept the demo auth-off + source-built (the `sdk-test.sh` pattern) so it's a self-contained
  dev tool, not a stranger-facing command; the README hero for strangers is still the
  auth-on quickstart.

## Test evidence

`scripts/lease-demo.sh` ran green end-to-end (Python + TypeScript workers, distinct claims,
drained-queue `null`); `bash -n` / `node --check` / `py_compile` / JSON-parse all pass;
mdbook linkcheck green.

## Forward look

Next: **318** token-pack evidence (the "far fewer tokens" number), then the **fidelity +
context flagship arc** — where the no-backwards-compat directive (rename `Reference.relation`
free-string → a controlled type, etc.) applies in full. A logged follow-up: publish the
quickstart image to GHCR for a true one-command no-clone eval.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the 2026-08-28 sweep
([[Open Work]]).
