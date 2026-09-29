"""Unit tests for retries, idempotency keys and auto-paging (no running server)."""

import io
import json
import urllib.error
from email.message import Message

import pytest

from maidan import Client, MaidanError, retry_delay

IN_FLIGHT = "https://maidan.dev/problems/idempotency-key-in-flight"


class _Resp:
    def __init__(self, status, body, headers):
        self.status, self._body, self.headers = status, body, headers

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def _headers(extra=None):
    m = Message()
    for k, v in (extra or {}).items():
        m[k] = v
    return m


def fake(answers, **kw):
    c = Client("http://x", "t", **kw)
    calls, sleeps = [], []

    def urlopen(req, timeout=None):
        calls.append(req)
        nxt = answers.pop(0)
        if isinstance(nxt, Exception):
            raise nxt
        status, body = nxt[0], json.dumps(nxt[1]).encode() if nxt[1] is not None else b""
        hdrs = _headers(nxt[2] if len(nxt) > 2 else None)
        if status >= 400:
            raise urllib.error.HTTPError(req.full_url, status, "x", hdrs, io.BytesIO(body))
        return _Resp(status, body, hdrs)

    c._urlopen = urlopen
    c._sleep = sleeps.append
    return c, calls, sleeps


def key(req):
    return req.get_header("Idempotency-key")


def test_write_retries_lost_response_with_same_key():
    c, calls, _ = fake([urllib.error.URLError("reset"), (201, {"id": "m1"})])
    assert c.messages.post("t1", "hi")["id"] == "m1"
    assert len(calls) == 2
    assert key(calls[0]) and key(calls[0]) == key(calls[1])


def test_each_write_gets_its_own_key_and_reads_none():
    c, calls, _ = fake([(201, {}), (201, {}), (200, [])])
    c.messages.post("t1", "a")
    c.messages.post("t1", "b")
    c.channels.list("w")
    assert key(calls[0]) != key(calls[1])
    assert key(calls[2]) is None


def test_429_retry_after_then_5xx_backoff_then_give_up():
    c, calls, sleeps = fake([(429, {}, {"Retry-After": "3"}), (503, {}), (503, {"detail": "down"})])
    with pytest.raises(MaidanError) as e:
        c.channels.list("w")
    assert e.value.status == 503
    assert len(calls) == 3
    assert sleeps[0] == 3.0
    assert 0.5 <= sleeps[1] <= 1.0


def test_in_flight_409_retried_plain_409_not():
    c, calls, _ = fake([(409, {"type": IN_FLIGHT}), (201, {"id": "c"})])
    assert c.channels.create("w", "n")["id"] == "c"
    assert len(calls) == 2
    c, calls, _ = fake([(409, {"type": "https://maidan.dev/problems/conflict"})])
    with pytest.raises(MaidanError):
        c.channels.create("w", "n")
    assert len(calls) == 1


def test_403_not_retried_and_max_retries_zero():
    c, calls, _ = fake([(403, {})])
    with pytest.raises(MaidanError):
        c.channels.list("w")
    assert len(calls) == 1
    c, calls, _ = fake([(503, {})], max_retries=0)
    with pytest.raises(MaidanError):
        c.channels.list("w")
    assert len(calls) == 1


def test_retry_delay():
    assert retry_delay(0, None, lambda: 0) == 0.25
    assert retry_delay(0, None, lambda: 1) == 0.5
    assert retry_delay(10, None, lambda: 1) == 8.0
    assert retry_delay(0, "120") == 60.0


def test_threads_list_all_pages_by_cursor():
    c, calls, _ = fake([(200, [{"id": "a"}, {"id": "b"}]), (200, [{"id": "c"}])])
    assert [t["id"] for t in c.threads.list_all("ch", page_size=2)] == ["a", "b", "c"]
    assert calls[0].full_url.endswith("/channels/ch/threads?limit=2")
    assert calls[1].full_url.endswith("limit=2&cursor=b")


def test_list_events_all_pages_by_after_id():
    c, calls, _ = fake([(200, [{"id": 1}, {"id": 2}]), (200, [{"id": 3}])])
    assert [e["id"] for e in c.list_events_all("w", {"limit": 2})] == [1, 2, 3]
    assert "after_id=2" in calls[1].full_url
