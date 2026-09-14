"""Unit tests for Room-LSN parse (no running server)."""

from maidan import event_type, parse_room_lsn


def test_parse_room_lsn_accepts_decimal_and_rejects_wal():
    assert parse_room_lsn("42") == 42
    assert parse_room_lsn(" 0 ") == 0
    assert parse_room_lsn("0/3000128") is None
    assert parse_room_lsn("-1") is None
    assert parse_room_lsn("") is None
    assert parse_room_lsn(None) is None
    assert event_type("message_posted") == "maidan.event.message_posted/1"
