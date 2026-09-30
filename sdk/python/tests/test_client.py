"""Black-box tests against the authenticated server from ``scripts/sdk-test.sh``."""

import json
import os
import threading
import urllib.request
import uuid

import pytest

import dataclasses

from maidan import (
    BadRequestError,
    ClaimedThread,
    Client,
    ConflictError,
    ForbiddenError,
    MaidanError,
    Model,
    NotFoundError,
    StrongRef,
    Thread,
    UnauthorizedError,
)

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
    thread = c.threads.create(channel.id, "kickoff")
    return c, ws, member, channel, thread


def test_hero_loop_post_list_context():
    c, _ws, member, _channel, thread = _seed()
    c.messages.post(thread.id, "hello from the py sdk")
    msgs = c.messages.list(thread.id)
    assert any(m.body == "hello from the py sdk" for m in msgs)
    ctx = c.threads.context(thread.id)
    assert ctx.thread.id == thread.id


def test_get_result_unset_is_404():
    # Exercise the result route and client error path before a result exists.
    c, _ws, _member, _channel, thread = _seed()
    with pytest.raises(NotFoundError) as ei:
        c.threads.get_result(thread.id)
    assert ei.value.status == 404


def test_claim_returns_the_thread_flattened_not_nested():
    """The seeded thread is ready, so this claims it. The shape assertions are the
    point: a nested ``thread`` key would make every README snippet a silent no-op."""
    c, _ws, member, channel, thread = _seed()
    claim = c.claim_next_thread(channel.id)
    assert claim is not None, "a freshly seeded ready thread should be claimable"
    assert isinstance(claim, ClaimedThread)
    assert "thread" not in claim.extra, "thread fields are flattened, not nested"
    assert claim.id == thread.id
    assert claim.assignee_id == member["id"]
    assert claim.claim_lease_id, "the fencing token renew_claim needs"
    assert isinstance(claim.pin, StrongRef)
    assert claim.pin.uri and claim.pin.content_hash


def test_renew_claim_extends_the_lease_with_the_fencing_token():
    c, _ws, member, channel, _thread = _seed()
    claim = c.claim_next_thread(channel.id, {"lease_secs": 60})
    renewed = c.renew_claim(claim.id, claim.claim_lease_id, 600)
    assert isinstance(renewed, Thread)
    assert renewed.assignment_expires_at > claim.assignment_expires_at


def test_claim_next_returns_null_when_nothing_is_ready():
    c, _ws, member, channel, _thread = _seed()
    c.claim_next_thread(channel.id)
    assert c.claim_next_thread(channel.id) is None


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
        if e.get("thread_id") == thread.id:
            received["event"] = e
            done.set()

    sub = c.subscribe({"workspace_id": ws["id"], "kinds": ["message_posted"]}, on_event)
    try:
        # Give the subscription a beat to attach, then post.
        threading.Timer(0.2, lambda: c.messages.post(thread.id, "ws ping")).start()
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
    assert member.handle == handle
    assert member.kind == "agent"
    assert any(m.id == member.id for m in c.members.list(WORKSPACE))

    minted = c.tokens.mint(WORKSPACE, member.id, ["workspace:read"], label="scoped worker")
    assert minted.secret, "the secret is returned once, in the mint response"
    assert minted.capabilities == ["workspace:read"]

    # Metadata only — listing never hands back a secret.
    listed = c.tokens.list(WORKSPACE, member.id)
    assert any(t.id == minted.id for t in listed)
    assert all("secret" not in t.extra for t in listed)
    assert all(t.label == "scoped worker" for t in listed if t.id == minted.id)


def test_threads_list_all_walks_every_page():
    c, _ws, _member, channel, thread = _seed()
    made = [thread.id] + [c.threads.create(channel.id, f"t{i}").id for i in range(4)]
    seen = [t.id for t in c.threads.list_all(channel.id, page_size=2)]
    assert sorted(seen) == sorted(made)


def _export():
    req = urllib.request.Request(
        f"{BASE}/workspaces/{WORKSPACE}/export", headers={"authorization": f"Bearer {TOKEN}"}
    )
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read())


def assert_modeled(value, path="response"):
    """Every member the server sent is declared on its model, all the way down.

    A model keeps undeclared members in ``extra`` (forward compatibility), so an
    empty ``extra`` everywhere is the proof that the models match what the live
    server returns. Required members are enforced by the dataclass itself."""
    if isinstance(value, list):
        for i, item in enumerate(value):
            assert_modeled(item, f"{path}[{i}]")
        return value
    if isinstance(value, Model):
        name = type(value).__name__
        assert value.extra == {}, f"{path} ({name}) got members its model does not declare: {sorted(value.extra)}"
        for f in dataclasses.fields(value):
            if f.name != "extra":
                assert_modeled(getattr(value, f.name), f"{path}.{f.name}")
    return value


def test_every_documented_operation_returns_its_declared_model():
    c, _ws, member, channel, thread = _seed()
    assert_modeled(c.workspaces.get(WORKSPACE))
    assert_modeled(c.members.list(WORKSPACE))
    assert_modeled(channel)
    assert_modeled(c.channels.list(WORKSPACE))
    assert_modeled(thread)
    assert_modeled(c.threads.get(thread.id))
    assert_modeled(c.threads.list(channel.id))

    msg = assert_modeled(c.messages.post(thread.id, "typed"))
    assert_modeled(c.messages.list(thread.id))

    art = assert_modeled(c.artifacts.upload(b"typed bytes", "attachment"))
    assert art.size_bytes == len(b"typed bytes")
    assert_modeled(c.artifacts.meta(art.sha256))
    assert c.artifacts.get(art.sha256) == b"typed bytes"

    claim = assert_modeled(c.claim_next_thread(channel.id, {"lease_secs": 60}))
    assert_modeled(c.renew_claim(claim.id, claim.claim_lease_id, 120))

    result = assert_modeled(c.threads.set_result(thread.id, {"ok": True}))
    assert result.result == {"ok": True}
    assert result.produced_by == member["id"]
    assert_modeled(c.threads.get_result(thread.id))
    reviewed = assert_modeled(c.threads.transition(thread.id, {"action": "start_review"}))
    assert reviewed.state == "in_review"

    ctx = assert_modeled(c.threads.context(thread.id))
    assert ctx.fsm.transitions, "the start_review transition is in the pack"
    assert any(m.id == msg.id for m in ctx.messages)

    events = assert_modeled(c.list_events(WORKSPACE, {"limit": 50}))
    assert events and all(e.type == f"maidan.event.{e.kind}/1" for e in events)
    assert_modeled(list(c.list_events_all(WORKSPACE, {"limit": 25})))

    fresh = assert_modeled(c.members.create(WORKSPACE, f"typed-{uuid.uuid4().hex}"))
    assert_modeled(c.tokens.mint(WORKSPACE, fresh.id, ["workspace:read"]))
    assert_modeled(c.tokens.list(WORKSPACE, fresh.id))

    imported = assert_modeled(c.workspaces.import_(_export(), "new"))
    assert imported.mode == "new"
    assert imported.workspace_id != WORKSPACE, "mode=new remaps ids"


@pytest.mark.parametrize(
    "call, cls, status, type_",
    [
        (lambda c, t: c.threads.get("00000000-0000-0000-0000-000000000000"), NotFoundError, 404, "not-found"),
        (lambda c, t: Client(BASE, "maid_not_a_token").workspaces.get(WORKSPACE), UnauthorizedError, 401, "unauthorized"),
        (lambda c, t: c.threads.transition(t.id, {"action": "fly"}), BadRequestError, 400, "bad-request"),
        # Bootstrap creates only the first workspace; `maidan init` already made it.
        (lambda c, t: c.workspaces.create("second"), ForbiddenError, 403, "forbidden"),
        (lambda c, t: c.workspaces.import_(_export(), "restore"), ConflictError, 409, "conflict"),
    ],
)
def test_the_servers_problem_types_arrive_as_their_error_classes(call, cls, status, type_):
    c, _ws, _member, _channel, thread = _seed()
    with pytest.raises(cls) as ei:
        call(c, thread)
    err = ei.value
    assert isinstance(err, MaidanError)
    assert err.status == status
    assert err.type == f"https://maidan.dev/problems/{type_}"
    assert err.problem["type"] == err.type, "the raw problem is kept"
    assert isinstance(err.title, str) and isinstance(err.detail, str)
