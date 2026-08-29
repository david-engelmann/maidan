"""Framework-independent Maidan client over plain REST (no MCP adapter).

REST is the most stable integration surface: it maps directly to the generated
OpenAPI document (GET /openapi.json). Use this shape from any language; here it
drives one agent turn (read the shared thread context, then post a reply).

    pip install "httpx>=0.27"

    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...      # from `maidan init`; the quickstart runs auth-on
    python examples/rest_maidan.py <thread_id> <member_id>
"""

import asyncio
import os
import sys

import httpx


class MaidanClient:
    def __init__(self, base_url: str, token: str | None = None) -> None:
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        self._client = httpx.AsyncClient(base_url=base_url.rstrip("/"), headers=headers, timeout=30.0)

    async def thread_context(self, thread_id: str) -> dict:
        resp = await self._client.get(f"/threads/{thread_id}/context")
        resp.raise_for_status()
        return resp.json()

    async def post_message(self, thread_id: str, author_id: str, body: str) -> dict:
        resp = await self._client.post(
            f"/threads/{thread_id}/messages",
            json={"author_id": author_id, "body": body},
        )
        resp.raise_for_status()
        return resp.json()

    async def aclose(self) -> None:
        await self._client.aclose()


async def main() -> None:
    if len(sys.argv) < 3:
        print("usage: rest_maidan.py <thread_id> <member_id>", file=sys.stderr)
        raise SystemExit(2)
    thread_id, member_id = sys.argv[1], sys.argv[2]

    client = MaidanClient(os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080"), os.environ.get("MAIDAN_TOKEN"))
    try:
        context = await client.thread_context(thread_id)
        print(f"thread has {len(context.get('messages', []))} messages")
        # Hand `context` to your model, then persist only the final reply:
        await client.post_message(thread_id, member_id, "Acknowledged; working on it.")
        print("posted reply")
    finally:
        await client.aclose()


if __name__ == "__main__":
    asyncio.run(main())
