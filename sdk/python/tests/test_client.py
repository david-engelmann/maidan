"""Black-box tests against the authenticated server from ``scripts/sdk-test.sh``."""

import json
import os
import threading
import urllib.request
import uuid

import pytest

from maidan import Client, MaidanError

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
TOKEN = os.environ.get("MAIDAN_TOKEN", "")
WORKSPACE = os.environ.get("MAIDAN_WORKSPACE", "")


def _client() -> Client:
    return Client(BASE, TOKEN)


def _me():
    req = urllib.request.Request(
        f"{BASE}/me", headers={"authorization": f"Bearer {TOKEN}"}
    )
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read())


def _seed():
    """Create an isolated queue in the token's bootstrap workspace."""
    c = _client()
    me = _me()
    ws = {"id": WORKSPACE}
    member = {"id": me["member_id"]}
    channel = c.channels.create(WORKSPACE, f"py-sdk-{uuid.uuid4().hex}")
    thread = c.threads.create(channel["id"], "kickoff")
    return c, ws, member, channel, thread


def test_hero_loop_post_list_context():
    c, _ws, member, _channel, thread = _seed()
    c.messages.post(thread["id"], "hello from the py sdk")
    msgs = c.messages.list(thread["id"])
    assert any(m["body"] == "hello from the py sdk" for m in msgs)
    ctx = c.threads.context(thread["id"])
    assert isinstance(ctx, dict)


def test_get_result_unset_is_404():
    # Exercise the result route and client error path before a result exists.
    c, _ws, _member, _channel, thread = _seed()
    with pytest.raises(MaidanError) as ei:
        c.threads.get_result(thread["id"])
    assert ei.value.status == 404


def test_claim_returns_the_thread_flattened_not_nested():
    """The seeded thread is ready, so this claims it. The shape assertions are the
    point: a nested ``thread`` key would make every README snippet a silent no-op."""
    c, _ws, member, channel, thread = _seed()
    claim = c.claim_next_thread(channel["id"])
    assert claim is not None, "a freshly seeded ready thread should be claimable"
    assert "thread" not in claim, "thread fields are flattened, not nested"
    assert claim["id"] == thread["id"]
    assert claim["assignee_id"] == member["id"]
    assert claim["claim_lease_id"], "the fencing token renew_claim needs"
    assert claim["pin"]["uri"] and claim["pin"]["content_hash"]


def test_renew_claim_extends_the_lease_with_the_fencing_token():
    c, _ws, member, channel, _thread = _seed()
    claim = c.claim_next_thread(
        channel["id"], {"lease_secs": 60}
    )
    renewed = c.renew_claim(claim["id"], claim["claim_lease_id"], 600)
    assert renewed["assignment_expires_at"] > claim["assignment_expires_at"]


def test_claim_next_returns_null_when_nothing_is_ready():
    c, _ws, member, channel, _thread = _seed()
    c.claim_next_thread(channel["id"])
    assert c.claim_next_thread(channel["id"]) is None


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
        threading.Timer(0.2, lambda: c.messages.post(thread["id"], "ws ping")).start()
        assert done.wait(10), "did not receive the message_posted event"
        assert received["event"]["kind"] == "message_posted"
    finally:
        sub.close()


def test_provisioning_seeds_a_member_and_mints_a_scoped_token():
    """The first thing an integrator does after `maidan init`: create an agent and
    mint it a narrow bearer. Both were reachable only through the private
    transport before, which is why the hero demo used `_req`."""
    c = _client()
    handle = f"provisioned-{uuid.uuid4().hex}"
    member = c.members.create(WORKSPACE, handle)
    assert member["handle"] == handle
    assert member["kind"] == "agent"
    assert any(m["id"] == member["id"] for m in c.members.list(WORKSPACE))

    minted = c.tokens.mint(
        WORKSPACE, member["id"], ["workspace:read"], label="scoped worker"
    )
    assert minted["secret"], "the secret is returned once, in the mint response"
    assert minted["capabilities"] == ["workspace:read"]

    # Metadata only — listing never hands back a secret.
    listed = c.tokens.list(WORKSPACE, member["id"])
    assert any(t["id"] == minted["id"] for t in listed)
    assert all("secret" not in t for t in listed)
