"""Connect LangChain to Maidan's MCP server — filtered to the hero task-loop tools.

Maidan speaks MCP over Streamable HTTP at POST /mcp/streamable. The full catalog is
~78 tools; handing all of them to an agent is expensive and noisy. This loads the
catalog, then **filters to the six-tool lease loop** an agent actually needs to pick
up work, do it, and hand back a result — then passes only those to your agent.

The filter is client-side: Maidan's catalog is unchanged, and the other tools stay
available if you widen HERO_TOOLS.

    # Pin mcp < 2: the 2.x SDK dropped `mcp.shared.session`, which
    # langchain-mcp-adapters 0.1.x still imports.
    pip install "langchain-mcp-adapters>=0.1,<0.2" "mcp>=1.9,<2"

    # Point at a running Maidan (e.g. the quickstart on http://127.0.0.1:8080).
    # The quickstart runs with auth on, so pass a bearer token (from `maidan init`).
    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...
    python examples/langchain_maidan.py
"""

import asyncio
import os

from langchain_mcp_adapters.client import MultiServerMCPClient

# The lease loop: claim work → read its context → post progress → record a result →
# block until a dependency's result / a task becomes ready. Widen as your agent needs.
HERO_TOOLS = {
    "claim_next_thread",
    "post_message",
    "get_thread_context",
    "set_thread_result",
    "wait_for_result",
    "wait_for_ready",
}


async def main() -> None:
    base_url = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
    connection: dict[str, object] = {
        "transport": "streamable_http",
        "url": f"{base_url}/mcp/streamable",
    }
    token = os.environ.get("MAIDAN_TOKEN")
    if token:
        connection["headers"] = {"Authorization": f"Bearer {token}"}

    client = MultiServerMCPClient({"maidan": connection})
    all_tools = await client.get_tools()
    tools = [t for t in all_tools if t.name in HERO_TOOLS]
    print(f"catalog has {len(all_tools)} tools; using the {len(tools)}-tool hero loop:")
    for tool in tools:
        print(f"  - {tool.name}")
    # Pass `tools` (not `all_tools`) to langchain.agents.create_agent(...) or a LangGraph node.


if __name__ == "__main__":
    asyncio.run(main())
