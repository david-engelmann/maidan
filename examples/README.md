# Examples

Runnable client examples for Maidan. Full write-up:
[docs/Framework Integrations.md](../docs/Framework%20Integrations.md).

Start a Maidan first (the quickstart runs one on `http://127.0.0.1:8080` with auth
disabled, so you can omit the token):

```sh
docker compose -f compose.quickstart.yaml up -d --build
```

| Example | What it shows | Install |
|---------|---------------|---------|
| [`langchain_maidan.py`](langchain_maidan.py) | Load Maidan's MCP tools into LangChain | `pip install "langchain-mcp-adapters>=0.1,<0.2" "mcp>=1.9,<2"` |
| [`autogen_maidan.py`](autogen_maidan.py) | Load Maidan's MCP tools into Microsoft AutoGen | `pip install "autogen-ext[mcp]>=0.4,<0.7" "mcp>=1.9,<2"` |
| [`rest_maidan.py`](rest_maidan.py) | Plain REST client (one agent turn) | `pip install "httpx>=0.27"` |
| [`a2a_interop.py`](a2a_interop.py) | A2A v1.0 conformance check (Agent Card + JSON-RPC + REST bindings) | `pip install "httpx>=0.27"` |

```sh
export MAIDAN_URL=http://127.0.0.1:8080
# export MAIDAN_TOKEN=maid_...   # only when auth is enabled
python examples/langchain_maidan.py
```

Pin `mcp < 2`: the 2.x SDK is not yet compatible with the current LangChain/AutoGen
MCP adapters. Give each agent its own capability-scoped token in production (see the
docs page).
