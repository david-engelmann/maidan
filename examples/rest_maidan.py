"""Framework-independent Maidan client over plain REST (no MCP adapter).

REST is the most stable integration surface: it maps directly to the generated
OpenAPI document (GET /openapi.json). Use this shape from any language; here it
drives one agent turn (read the shared thread context, then post a reply).

    pip install "httpx>=0.27"

    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...      # from `maidan init`; the quickstart runs auth-on
    # Self-seed from the workspace/member carried by the admin token:
    python examples/rest_maidan.py --seed

    # Or operate on an existing thread as the token-bound member:
    python examples/rest_maidan.py <thread_id>
"""

import asyncio
import ipaddress
import os
import sys
import urllib.parse
import uuid

import httpx

USER_AGENT = "maidan-rest-example/0.1"


def _loopback(host: str) -> bool:
    host = host.strip().lower()
    if host.startswith("[") and host.endswith("]"):
        host = host[1:-1]
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def refuse_cleartext(url: str, token: str | None) -> None:
    if not token or os.environ.get("MAIDAN_ALLOW_INSECURE_HTTP") == "1":
        return
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "http" or _loopback(parsed.hostname or ""):
        return
    print(
        f"refusing to send MAIDAN_TOKEN over cleartext to {url}; "
        "use https, or set MAIDAN_ALLOW_INSECURE_HTTP=1 for a trusted network",
        file=sys.stderr,
    )
    raise SystemExit(2)


class MaidanClient:
    def __init__(self, base_url: str, token: str | None = None) -> None:
        headers = {"user-agent": USER_AGENT}
        if token:
            headers["authorization"] = f"Bearer {token}"
        self._client = httpx.AsyncClient(base_url=base_url, headers=headers, timeout=30.0)

    def _write_headers(self) -> dict[str, str]:
        return {"idempotency-key": str(uuid.uuid4()), "user-agent": USER_AGENT}

    async def thread_context(self, thread_id: str) -> dict:
        resp = await self._client.get(f"/threads/{thread_id}/context")
        resp.raise_for_status()
        return resp.json()

    async def whoami(self) -> dict:
        resp = await self._client.get("/me")
        resp.raise_for_status()
        return resp.json()

    async def create_channel(self, workspace_id: str, name: str) -> dict:
        resp = await self._client.post(
            f"/workspaces/{workspace_id}/channels",
            json={"name": name, "private": False},
            headers=self._write_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    async def create_thread(self, channel_id: str, title: str) -> dict:
        resp = await self._client.post(
            f"/channels/{channel_id}/threads",
            json={"title": title},
            headers=self._write_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    async def post_message(self, thread_id: str, body: str) -> dict:
        resp = await self._client.post(
            f"/threads/{thread_id}/messages",
            json={"body": body},
            headers=self._write_headers(),
        )
        resp.raise_for_status()
        return resp.json()

    async def aclose(self) -> None:
        await self._client.aclose()


async def main() -> None:
    args = sys.argv[1:]
    base = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080").strip().rstrip("/")
    token = os.environ.get("MAIDAN_TOKEN")
    refuse_cleartext(base, token)
    run_id = uuid.uuid4().hex[:8]
    client = MaidanClient(base, token)
    try:
        if args == ["--seed"]:
            me = await client.whoami()
            workspace_id = me["workspace_id"]
            channel = await client.create_channel(workspace_id, f"rest-example-{run_id}")
            thread = await client.create_thread(channel["id"], f"REST example ({run_id}): one agent turn")
            thread_id = thread["id"]
            print(f"seeded channel={channel['id']} thread={thread_id} member={me['member_id']}")
        elif len(args) == 1:
            thread_id = args[0]
        else:
            print("usage: rest_maidan.py --seed | <thread_id>", file=sys.stderr)
            raise SystemExit(2)

        context = await client.thread_context(thread_id)
        print(f"thread has {len(context.get('messages', []))} messages")
        # Hand `context` to your model, then persist only the final reply:
        await client.post_message(thread_id, "Acknowledged; working on it.")
        print("posted reply")
    finally:
        await client.aclose()


if __name__ == "__main__":
    asyncio.run(main())
