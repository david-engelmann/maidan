"""Black-box test for the Maidan Python client against a running server
(MAIDAN_URL, auth disabled). Run via ``scripts/sdk-test.sh python``, which boots a
server. These scenarios also exercise the server's REST + WS surface.
"""

import json
import os
import threading
import urllib.request

import pytest

from maidan import Client, MaidanError

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")


def _client() -> Client:
    return Client(BASE, os.environ.get("MAIDAN_TOKEN", ""))


def _seed():
    """Member creation isn't in the SDK surface (seeded via bootstrap/CLI); seed
    one over the raw bootstrap route."""
    c = _client()
    ws = c.workspaces.create("py-sdk")
    req = urllib.request.Request(
        f"{BASE}/workspaces/{ws['id']}/members",
        data=json.dumps({"handle": "sdk-agent", "kind": "agent"}).encode(),
        method="POST",
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(req) as resp:
        member = json.loads(resp.read())
    channel = c.channels.create(ws["id"], "general")
    thread = c.threads.create(channel["id"], "kickoff")
    return c, ws, member, channel, thread


def test_hero_loop_post_list_context():
    c, _ws, member, _channel, thread = _seed()
    c.messages.post(thread["id"], member["id"], "hello from the py sdk")
    msgs = c.messages.list(thread["id"])
    assert any(m["body"] == "hello from the py sdk" for m in msgs)
    ctx = c.threads.context(thread["id"])
    assert isinstance(ctx, dict)


def test_get_result_unset_is_404():
    # A full set_result round-trip needs a real produced_by member (auth-enabled;
    # the server's thread_result_e2e proves it). Under the auth-disabled harness the
    # acting member is nil, so here we exercise the result route + client error path.
    c, _ws, _member, _channel, thread = _seed()
    with pytest.raises(MaidanError) as ei:
        c.threads.get_result(thread["id"])
    assert ei.value.status == 404


def test_claim_next_returns_claimable_or_null():
    c, _ws, member, channel, _thread = _seed()
    res = c.claim_next_thread(channel["id"], {"member_id": member["id"]})
    assert res is None or isinstance(res, dict)


def test_errors_surface_status_and_body():
    c = _client()
    with pytest.raises(MaidanError) as ei:
        c.threads.get("00000000-0000-0000-0000-000000000000")
    assert ei.value.status >= 400


def test_subscribe_delivers_a_posted_message():
    c, ws, member, _channel, thread = _seed()
    received: dict = {}
    done = threading.Event()

    def on_event(e):
        if e.get("thread_id") == thread["id"]:
            received["event"] = e
            done.set()

    sub = c.subscribe({"workspace_id": ws["id"], "kinds": ["message_posted"]}, on_event)
    try:
        # Give the subscription a beat to attach, then post.
        threading.Timer(0.2, lambda: c.messages.post(thread["id"], member["id"], "ws ping")).start()
        assert done.wait(10), "did not receive the message_posted event"
        assert received["event"]["kind"] == "message_posted"
    finally:
        sub.close()
