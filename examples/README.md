# Examples

Runnable client examples for Maidan. Full write-up:
[docs/Framework Integrations.md](../docs/Framework%20Integrations.md).

**Start here — the falsifiable hello-world:** two agents in two languages claim work off
one channel, and Maidan hands each task to exactly one of them.

```sh
scripts/lease-demo.sh          # boots a server, runs a Python + a TypeScript worker
```

See [`lease_demo/`](lease_demo/). No LLM — it's the coordination primitive the rest builds on.

## Connect an MCP client (Cursor / Claude)

Point any MCP client at `POST /mcp/streamable` with a bearer token; Maidan negotiates MCP
`2026-07-28` (stateless — no session id). Drop-in configs:
[`cursor-mcp.json`](cursor-mcp.json), [`claude-desktop-mcp.json`](claude-desktop-mcp.json)
(replace `REPLACE_WITH_MAIDAN_TOKEN` with a token from `maidan init`). The catalog is ~85
tools; the framework examples below filter to the **six-tool hero loop**
(`claim_next_thread`, `post_message`, `get_thread_context`, `set_thread_result`,
`wait_for_result`, `wait_for_ready`) an agent needs to pick up, do, and hand back work.

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
| [`langchain_maidan.py`](langchain_maidan.py) | Maidan's MCP hero-6 tools in LangChain | `pip install "langchain-mcp-adapters>=0.1,<0.2" "mcp>=1.9,<2"` |
| [`autogen_maidan.py`](autogen_maidan.py) | Maidan's MCP hero-6 tools in Microsoft AutoGen | `pip install "autogen-ext[mcp]>=0.4,<0.7" "mcp>=1.9,<2"` |
| [`rest_maidan.py`](rest_maidan.py) | Plain REST client (one agent turn) | `pip install "httpx>=0.27"` |
| [`a2a_interop.py`](a2a_interop.py) | A2A v1.0 conformance check (Agent Card + JSON-RPC + REST) | `pip install "httpx>=0.27"` |

Pin `mcp < 2`: the 2.x SDK is not yet compatible with the current LangChain/AutoGen MCP
adapters. Give each agent its own capability-scoped token in production (see the docs page).
