"""Connect Microsoft AutoGen to Maidan's MCP server — filtered to the hero task-loop.

Maidan speaks MCP over Streamable HTTP at POST /mcp/streamable. This loads the catalog
via autogen-ext's MCP adapter, then **filters to the seven-tool lease loop** before
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
import sys

from autogen_ext.tools.mcp import StreamableHttpServerParams, mcp_server_tools

HERO_TOOLS = {
    "claim_next_thread",
    "post_message",
    "get_thread_context",
    "set_thread_result",
    "transition_thread",
    "wait_for_result",
    "wait_for_ready",
}


def report(all_tools, tools) -> int:
    """Print what the agent will get, and fail loudly if that is not the hero loop.

    `tools/list` is capability-filtered server-side: a token that lacks
    `thread:transition` or `message:post` simply does not see `claim_next_thread`
    or `post_message`. Filtering an already-filtered catalog then yields a short
    list, and an example that prints it and exits 0 hands you a broken agent with
    a clean run.
    """
    missing = HERO_TOOLS - {t.name for t in tools}
    print(f"catalog has {len(all_tools)} tools; the hero loop needs {len(HERO_TOOLS)}:")
    for name in sorted(HERO_TOOLS):
        print(f"  {'ok' if name not in missing else 'MISSING':<9}{name}")
    if missing:
        print(
            f"\n{len(missing)} hero tool(s) missing: {', '.join(sorted(missing))}.\n"
            "The MCP catalog is filtered to what your token may invoke, so this is "
            "almost always the token — mint one with `workspace:read`, `message:post` "
            "and `thread:transition` (or the `maidan.agent.worker` set).",
            file=sys.stderr,
        )
        return 1
    print("\nwiring ok — pass `tools` (not `all_tools`) to your agent.")
    return 0


async def main() -> int:
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
    return report(all_tools, tools)


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
