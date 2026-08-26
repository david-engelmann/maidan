"""Connect Microsoft AutoGen to Maidan's MCP server and load its tools.

Maidan speaks MCP over Streamable HTTP at POST /mcp/streamable. This uses
autogen-ext's MCP adapter to pull Maidan's tools into an AutoGen agent. Give
each agent its own Maidan member and a capability-scoped bearer token in
production.

    # Pin mcp < 2: the 2.x SDK is not yet compatible with these adapters.
    pip install "autogen-ext[mcp]>=0.4,<0.7" "mcp>=1.9,<2"

    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...      # omit against the auth-disabled quickstart
    python examples/autogen_maidan.py
"""

import asyncio
import os

from autogen_ext.tools.mcp import StreamableHttpServerParams, mcp_server_tools


async def main() -> None:
    base_url = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
    token = os.environ.get("MAIDAN_TOKEN")
    headers = {"Authorization": f"Bearer {token}"} if token else None

    params = StreamableHttpServerParams(
        url=f"{base_url}/mcp/streamable",
        headers=headers,
        timeout=30.0,
        sse_read_timeout=300.0,
    )
    tools = await mcp_server_tools(params)
    print(f"loaded {len(tools)} Maidan tools:")
    for tool in tools:
        print(f"  - {tool.name}")
    # Pass `tools` to autogen_agentchat.agents.AssistantAgent(...).


if __name__ == "__main__":
    asyncio.run(main())
