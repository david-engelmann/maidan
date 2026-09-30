"""Unit tests for cursor-too-old detection (no running server)."""

from maidan import CursorTooOldError, MaidanError, StoredEvent, problem_error
from maidan.client import _normalize_stored


def test_is_cursor_too_old_requires_409_must_refetch():
    too_old = MaidanError(
        409,
        {
            "type": "https://maidan.dev/problems/cursor-too-old",
            "must_refetch": True,
        },
    )
    assert too_old.is_conflict
    assert too_old.is_cursor_too_old

    plain = MaidanError(409, {"type": "https://maidan.dev/problems/conflict"})
    assert plain.is_conflict
    assert not plain.is_cursor_too_old

    assert not MaidanError(500, {"must_refetch": True}).is_cursor_too_old

    typed = problem_error(
        409,
        {
            "type": "https://maidan.dev/problems/cursor-too-old",
            "must_refetch": True,
            "snapshot": "/workspaces/w/snapshot",
        },
    )
    assert isinstance(typed, CursorTooOldError)
    assert typed.is_cursor_too_old
    assert typed.snapshot == "/workspaces/w/snapshot"


def test_normalize_stored_promotes_id():
    live = _normalize_stored(
        StoredEvent.from_dict(
            {
                "$type": "maidan.event.message_posted/1",
                "id": 42,
                "lsn": 42,
                "kind": "message_posted",
                "workspace_id": "ws",
                "payload": {"kind": "message_posted", "body": "hi"},
                "occurred_at": "2026-09-29T00:00:00Z",
                "prev_hash": "p",
                "content_hash": "c",
            }
        )
    )
    assert live["log_id"] == 42
    assert live["body"] == "hi"
    assert live["kind"] == "message_posted"
    assert live["workspace_id"] == "ws"
