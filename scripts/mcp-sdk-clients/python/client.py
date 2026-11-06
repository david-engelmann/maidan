"""The official MCP Python SDK as a client of a seeded Maidan.

Usage: client.py <base-url> <endpoint> <bearer-token>
Connects over Streamable HTTP, lists the tools, then walks the room the way an
agent would: whoami, create a channel and a thread, post a message, and read it
back. Exits non-zero on the first check that fails.

Argument checks run before the SDK is imported so a bad URL/token fails closed
without needing a venv install.
"""

from __future__ import annotations

import json
import sys
from urllib.parse import urlparse

ALLOWED_ENDPOINTS = frozenset({"/mcp", "/mcp/streamable"})


def fail(msg: str) -> None:
    print(f"FAIL (python): {msg}", file=sys.stderr)
    sys.exit(1)


def assert_client_args(base: str, endpoint: str, token: str) -> None:
    """Refuse non-loopback cleartext and non-http(s) bases; allowlist MCP paths."""
    if not base or not endpoint or not token:
        fail("usage: client.py <base-url> <endpoint> <bearer-token>")
    parsed = urlparse(base)
    if parsed.scheme not in ("http", "https"):
        fail(f"base-url must be http(s): {base}")
    if parsed.scheme == "http":
        host = (parsed.hostname or "").lower()
        if host not in ("127.0.0.1", "localhost", "::1"):
            fail(f"refusing cleartext base-url off loopback: {base}")
    if parsed.username or parsed.password:
        fail(f"base-url must not embed credentials: {base}")
    if endpoint not in ALLOWED_ENDPOINTS:
        fail(f"endpoint must be /mcp or /mcp/streamable (got {endpoint})")
    if not isinstance(token, str) or len(token.strip()) < 8 or any(ch.isspace() for ch in token):
        fail("bearer-token looks empty or malformed")


async def _run(base: str, endpoint: str, token: str) -> None:
    import asyncio

    from mcp import ClientSession
    from mcp.client.streamable_http import create_mcp_http_client, streamable_http_client

    async def call(session: ClientSession, tool: str, args: dict) -> dict:
        result = await session.call_tool(tool, args)
        if getattr(result, "isError", False) or getattr(result, "is_error", False):
            fail(f"{tool} {args}: {result}")
        return json.loads(result.content[0].text)

    http = create_mcp_http_client(headers={"Authorization": f"Bearer {token}"})
    async with http, streamable_http_client(base + endpoint, http_client=http) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            print("  ok  initialize")
            tools = (await session.list_tools()).tools
            if len(tools) < 100:
                fail(f"tools/list returned {len(tools)} tools")
            print(f"  ok  tools/list: {len(tools)} tools")
            me = await call(session, "whoami", {})
            ws = me["workspace_id"]
            channel = (
                await call(
                    session,
                    "create_channel",
                    {"workspace_id": ws, "name": "sdk-python" + endpoint.replace("/", "-")},
                )
            )["id"]
            thread = (await call(session, "create_thread", {"channel_id": channel, "title": "from the python sdk"}))[
                "id"
            ]
            await call(session, "post_message", {"thread_id": thread, "body": "hello from python"})
            listed = json.dumps(await call(session, "list_messages", {"thread_id": thread}))
            if "hello from python" not in listed:
                fail(f"list_messages did not return the posted message: {listed}")
            print("  ok  whoami, create_channel, create_thread, post_message, list_messages")
            try:
                bad = await session.call_tool("get_thread_context", {"thread_id": "not-a-uuid"})
                if not (getattr(bad, "isError", False) or getattr(bad, "is_error", False)):
                    fail("an invalid thread_id was accepted")
                refusal = str(bad.content)
            except Exception as err:  # noqa: BLE001 - the SDK raises its own McpError
                refusal = str(err)
            if "thread_id" not in refusal:
                fail(f"the refusal does not name thread_id: {refusal}")
            print("  ok  an invalid argument is refused, naming it")


def main(argv: list[str] | None = None) -> None:
    import asyncio

    args = list(sys.argv[1:] if argv is None else argv)
    if len(args) != 3:
        fail("usage: client.py <base-url> <endpoint> <bearer-token>")
    base, endpoint, token = args
    assert_client_args(base, endpoint, token)
    asyncio.run(_run(base, endpoint, token))


if __name__ == "__main__":
    main()
