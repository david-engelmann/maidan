"""Connect LangChain to Maidan's MCP server and load its tools.

Maidan speaks MCP over Streamable HTTP at POST /mcp/streamable. This uses the
official langchain-mcp-adapters client to pull Maidan's tools into a LangChain
agent. Give each agent its own Maidan member and a capability-scoped bearer
token in production.

    # Pin mcp < 2: the 2.x SDK dropped `mcp.shared.session`, which
    # langchain-mcp-adapters 0.1.x still imports.
    pip install "langchain-mcp-adapters>=0.1,<0.2" "mcp>=1.9,<2"

    # Point at a running Maidan (e.g. the quickstart on http://127.0.0.1:8080).
    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...      # omit against the auth-disabled quickstart
    python examples/langchain_maidan.py
"""

import asyncio
import os

from langchain_mcp_adapters.client import MultiServerMCPClient


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
    tools = await client.get_tools()
    print(f"loaded {len(tools)} Maidan tools:")
    for tool in tools:
        print(f"  - {tool.name}")
    # Pass `tools` to langchain.agents.create_agent(...) or a LangGraph node.


if __name__ == "__main__":
    asyncio.run(main())
