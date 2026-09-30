"""Unit tests for the problem-type errors and forward-compatible models (no server)."""

import io
import json
import urllib.error
from email.message import Message

import pytest

import maidan
from maidan import (
    PROBLEM_BASE,
    PROBLEM_TYPES,
    Client,
    MaidanError,
    NotFoundError,
    OverloadedError,
    Thread,
    ThreadContext,
    UnknownProblemError,
    problem_error,
)

SERVER_TYPES = [
    "not-found",
    "method-not-allowed",
    "conflict",
    "bad-request",
    "unauthorized",
    "invalid-signature",
    "forbidden",
    "payload-too-large",
    "unsupported-media-type",
    "rate-limited",
    "bad-gateway",
    "internal",
    "overloaded",
    "idempotency-key-reused",
    "idempotency-key-in-flight",
    "cursor-too-old",
    "event-log-broken",
]


def answering(status, body, headers=None):
    c = Client("http://x", "t", max_retries=0)
    raw = body.encode() if isinstance(body, str) else json.dumps(body).encode()

    def urlopen(req, timeout=None):
        hdrs = Message()
        for k, v in (headers or {}).items():
            hdrs[k] = v
        raise urllib.error.HTTPError(req.full_url, status, "x", hdrs, io.BytesIO(raw))

    c._urlopen = urlopen
    return c


def test_every_problem_type_the_server_documents_has_its_own_class():
    assert sorted(PROBLEM_TYPES) == sorted(PROBLEM_BASE + t for t in SERVER_TYPES)
    assert len(set(PROBLEM_TYPES.values())) == len(SERVER_TYPES)
    for type_, cls in PROBLEM_TYPES.items():
        err = problem_error(418, {"type": type_, "title": "T", "status": 418, "detail": "d"})
        assert type(err) is cls and isinstance(err, MaidanError)
        assert cls is not UnknownProblemError
        assert getattr(maidan, cls.__name__) is cls, "exported from the package"


def test_an_error_carries_status_type_title_detail_and_the_raw_problem():
    body = {
        "type": f"{PROBLEM_BASE}not-found",
        "title": "Not Found",
        "status": 404,
        "detail": "the requested resource does not exist",
        "trace": "a member added later",
    }
    with pytest.raises(NotFoundError) as ei:
        answering(404, body).threads.get("t1")
    err = ei.value
    assert (err.status, err.type, err.title, err.detail) == (404, body["type"], "Not Found", body["detail"])
    assert err.problem == body
    assert "HTTP 404: the requested resource does not exist" in str(err)


def test_an_unknown_problem_type_falls_back_to_unknown_problem_error():
    type_ = f"{PROBLEM_BASE}added-next-year"
    with pytest.raises(UnknownProblemError) as ei:
        answering(409, {"type": type_, "title": "New", "status": 409, "detail": "d"}).threads.get("t")
    assert ei.value.type == type_
    assert ei.value.is_conflict


def test_a_body_that_is_not_a_problem_is_unknown_with_its_text_as_detail():
    with pytest.raises(UnknownProblemError) as ei:
        answering(502, "<html>bad gateway</html>").threads.get("t")
    assert ei.value.type is None
    assert ei.value.problem is None
    assert ei.value.detail == "<html>bad gateway</html>"


def test_retry_after_is_kept_on_an_overloaded_503():
    body = {"type": f"{PROBLEM_BASE}overloaded", "title": "Service Unavailable", "status": 503, "detail": "busy"}
    with pytest.raises(OverloadedError) as ei:
        answering(503, body, {"Retry-After": "7"}).threads.get("t")
    assert ei.value.retry_after == 7.0


def test_members_a_model_does_not_declare_are_kept_in_extra():
    t = Thread.from_dict(
        {
            "id": "t1",
            "channel_id": "c1",
            "state": "blocked_on_mars",
            "created_at": "x",
            "updated_at": "x",
            "novel": {"deep": 1},
        }
    )
    assert t.id == "t1"
    assert t.state == "blocked_on_mars", "an enum value this client does not know passes through"
    assert t.extra == {"novel": {"deep": 1}}
    assert t.title is None


def test_nested_models_decode_and_missing_required_members_fail_loud():
    ts = "2026-09-29T00:00:00Z"
    th = {"id": "t1", "channel_id": "c1", "state": "open", "created_at": ts, "updated_at": ts}
    ctx = ThreadContext.from_dict(
        {
            "workspace_id": "w",
            "channel_id": "c1",
            "thread": th,
            "messages": [{"id": "m", "thread_id": "t1", "author_id": "a", "body": "b", "posted_at": ts}],
            "message_edits": [],
            "references": [],
            "artifacts": [],
            "fsm": {"state": "open", "transitions": []},
        }
    )
    assert isinstance(ctx.thread, Thread)
    assert ctx.messages[0].body == "b"
    assert ctx.glossary == []
    with pytest.raises(ValueError, match="Thread"):
        Thread.from_dict({"id": "t1"})
