"""Connect LangChain to Maidan's MCP server — filtered to the hero task-loop tools.

Maidan speaks MCP over Streamable HTTP at POST /mcp/streamable. The catalog is large — handing all of it to an agent is expensive and noisy. This loads the
catalog, then **filters to the seven-tool lease loop** an agent actually needs to pick
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
import sys

from langchain_mcp_adapters.client import MultiServerMCPClient

# The lease loop: claim work → read its context → post progress → record a result →
# hand it to review (`transition_thread` `start_review`, or the task is claimed again) →
# block until a dependency's result / a task becomes ready. Widen as your agent needs.
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
    return report(all_tools, tools)


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
