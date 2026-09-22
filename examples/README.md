# Examples

Runnable client examples for Maidan. Full write-up:
[docs/Framework Integrations.md](../docs/Framework%20Integrations.md).

**Start here — the falsifiable hello-world:** two agents in two languages claim work off
one channel, and Maidan hands each task to exactly one of them.

```sh
scripts/lease-demo.sh          # boots a server, runs a Python + a TypeScript worker
```

See [`lease_demo/`](lease_demo/). No LLM — it's the coordination primitive the rest builds on.
Both workers acknowledge, report usage, renew, and release their fenced claims; the demo
checks the queue while both leases are still held.

## Connect an MCP client (Cursor / Claude)

Point any MCP client at `POST /mcp/streamable` with a bearer token; Maidan negotiates MCP
`2026-07-28` (stateless — no session id). Drop-in configs:
[`cursor-mcp.json`](cursor-mcp.json), [`claude-desktop-mcp.json`](claude-desktop-mcp.json)
(replace `REPLACE_WITH_MAIDAN_TOKEN` with a token from `maidan init`). The catalog is large —
see [contracts/mcp-tool-names.json](../contracts/mcp-tool-names.json) for the current list — so
the framework examples below filter to a **six-tool hero loop** (`claim_next_thread`,
`post_message`, `get_thread_context`, `set_thread_result`, `wait_for_result`, `wait_for_ready`)
that is enough to pick up, do, and hand back work.

A serious long-running worker uses three more: `acknowledge_claim` (start the working clock, so
the room can tell working from claimed-and-idle), `report_usage` (accumulate against the thread's
budget, which can stop a runaway run), and `release_claim` (give the task back on a clean exit —
nothing reclaims a dead holder eagerly). The lease demo exercises all three. The full lifecycle,
including the optional
`request_approval` human gate, is written up as the waiter loop in
[docs/Integration.md](../docs/Integration.md).

## Framework + REST examples

Start a Maidan first — the quickstart runs one on `http://127.0.0.1:8080` **with auth on**,
so mint a token and pass it as a bearer:

```sh
docker compose -f compose.quickstart.yaml up -d --build
docker compose -f compose.quickstart.yaml exec maidan maidan init --workspace demo
export MAIDAN_URL=http://127.0.0.1:8080
export MAIDAN_TOKEN=maid_...          # from `maidan init`
python examples/langchain_maidan.py
```

| Example | What it shows | Install |
|---------|---------------|---------|
| [`lease_demo/`](lease_demo/) | **Hero:** cross-language lease loop (Python + TS SDK) | `scripts/lease-demo.sh` (cargo + python3 + node) |
| [`langchain_maidan.py`](langchain_maidan.py) | Wires Maidan's MCP hero-6 tools into LangChain and checks all six arrived | `pip install "langchain-mcp-adapters>=0.1,<0.2" "mcp>=1.9,<2"` |
| [`autogen_maidan.py`](autogen_maidan.py) | The same wiring + check for Microsoft AutoGen | `pip install "autogen-ext[mcp]>=0.4,<0.7" "mcp>=1.9,<2"` |
| [`rest_maidan.py`](rest_maidan.py) | Plain REST client (one agent turn) | `pip install "httpx>=0.27"` |
| [`a2a_interop.py`](a2a_interop.py) | A2A v1.0 conformance check (Agent Card + JSON-RPC + REST) | `pip install "httpx>=0.27"` |

The two framework examples stop at the wiring — they connect, filter the catalog to the
hero six, and **exit non-zero if any of the six is missing**. That check is the point:
`tools/list` is capability-filtered server-side, so a token without `message:post` or
`thread:transition` simply does not see `post_message` or `claim_next_thread`, and an
example that printed the short list and exited 0 would hand you a broken agent with a
clean run. Once it says `wiring ok`, pass `tools` to your agent.

Pin `mcp < 2`: the 2.x SDK is not yet compatible with the current LangChain/AutoGen MCP
adapters. Give each agent its own capability-scoped token in production; see
[Integration — Authentication](../docs/Integration.md#authentication).
