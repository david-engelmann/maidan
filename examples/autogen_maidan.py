"""Connect Microsoft AutoGen to Maidan's MCP server — filtered to the hero task-loop.

Maidan speaks MCP over Streamable HTTP at POST /mcp/streamable. This loads the catalog
via autogen-ext's MCP adapter, then **filters to the six-tool lease loop** before
handing tools to an agent — the full ~78-tool catalog is unchanged server-side.

    # Pin mcp < 2: the 2.x SDK is not yet compatible with these adapters.
    pip install "autogen-ext[mcp]>=0.4,<0.7" "mcp>=1.9,<2"

    # The quickstart runs with auth on, so pass a bearer token (from `maidan init`).
    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...
    python examples/autogen_maidan.py
"""

import asyncio
import os

from autogen_ext.tools.mcp import StreamableHttpServerParams, mcp_server_tools

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
    token = os.environ.get("MAIDAN_TOKEN")
    headers = {"Authorization": f"Bearer {token}"} if token else None

    params = StreamableHttpServerParams(
        url=f"{base_url}/mcp/streamable",
        headers=headers,
        timeout=30.0,
        sse_read_timeout=300.0,
    )
    all_tools = await mcp_server_tools(params)
    tools = [t for t in all_tools if t.name in HERO_TOOLS]
    print(f"catalog has {len(all_tools)} tools; using the {len(tools)}-tool hero loop:")
    for tool in tools:
        print(f"  - {tool.name}")
    # Pass `tools` (not `all_tools`) to autogen_agentchat.agents.AssistantAgent(...).


if __name__ == "__main__":
    asyncio.run(main())
