"""Errors: one class per RFC 9457 problem ``type`` the server documents.

Every failed response raises a :class:`MaidanError` subclass chosen by the
problem body's ``type``. A type this client does not know, or a body that is not
a problem at all (a proxy's HTML page), raises :class:`UnknownProblemError`, so
``except MaidanError`` still catches everything.
"""

from __future__ import annotations

from typing import Any, Dict, Optional, Type

PROBLEM_BASE = "https://maidan.dev/problems/"


def _str(value: Any) -> Optional[str]:
    return value if isinstance(value, str) else None


class MaidanError(Exception):
    """A failed request.

    ``type``, ``title`` and ``detail`` come from the problem body; ``problem`` is
    that body as sent, unknown members included (``None`` when the body was not a
    JSON object, in which case ``detail`` holds its text). ``status`` 0 is a
    failure with no HTTP answer.
    """

    def __init__(self, status: int, problem: Any = None, message: Optional[str] = None):
        p = problem if isinstance(problem, dict) else None
        detail = _str(p.get("detail")) if p is not None else _str(problem)
        super().__init__(message or f"Maidan request failed: HTTP {status}" + (f": {detail}" if detail else ""))
        self.status = status
        self.type: Optional[str] = _str(p.get("type")) if p is not None else None
        self.title: Optional[str] = _str(p.get("title")) if p is not None else None
        self.detail: Optional[str] = detail
        self.problem: Optional[Dict[str, Any]] = p
        # Seconds from Retry-After, sent on 429 (rate limit) and 503 (overloaded).
        self.retry_after: Optional[float] = None

    @property
    def is_conflict(self) -> bool:  # 409
        return self.status == 409

    @property
    def is_cursor_too_old(self) -> bool:
        """409 + must_refetch / cursor-too-old — fail loud, never clamp."""
        if self.status != 409:
            return False
        if self.problem is not None and self.problem.get("must_refetch") is True:
            return True
        return self.type in (f"{PROBLEM_BASE}cursor-too-old", "cursor_too_old")

    @property
    def is_forbidden(self) -> bool:  # 403 (missing capability / channel access)
        return self.status == 403

    @property
    def is_rate_limited(self) -> bool:  # 429
        return self.status == 429


class NotFoundError(MaidanError):
    """404 ``not-found``."""


class MethodNotAllowedError(MaidanError):
    """405 ``method-not-allowed``."""


class ConflictError(MaidanError):
    """409 ``conflict``: the resource's state refuses the change."""


class BadRequestError(MaidanError):
    """400 ``bad-request``."""


class UnauthorizedError(MaidanError):
    """401 ``unauthorized``: missing or invalid bearer token."""


class InvalidSignatureError(MaidanError):
    """401 ``invalid-signature`` (webhook ingress)."""


class ForbiddenError(MaidanError):
    """403 ``forbidden``: a missing capability or channel access. Not retryable."""


class PayloadTooLargeError(MaidanError):
    """413 ``payload-too-large``."""


class UnsupportedMediaTypeError(MaidanError):
    """415 ``unsupported-media-type``."""


class RateLimitedError(MaidanError):
    """429 ``rate-limited``; see ``retry_after``."""


class BadGatewayError(MaidanError):
    """502 ``bad-gateway``."""


class InternalError(MaidanError):
    """500 ``internal``."""


class OverloadedError(MaidanError):
    """503 ``overloaded``: refused without running; retry after ``retry_after``."""


class IdempotencyKeyReusedError(MaidanError):
    """422 ``idempotency-key-reused``: the key was used for a different request."""


class IdempotencyKeyInFlightError(MaidanError):
    """409 ``idempotency-key-in-flight``: the first request with the key still runs."""


class CursorTooOldError(MaidanError):
    """409 ``cursor-too-old``: refetch from ``snapshot``, never clamp the cursor."""

    @property
    def snapshot(self) -> Optional[str]:
        return _str(self.problem.get("snapshot")) if self.problem is not None else None


class EventLogBrokenError(MaidanError):
    """409 ``event-log-broken``: the hash chain failed verification."""


class UnknownProblemError(MaidanError):
    """A problem ``type`` this client does not know, or a body that is not a problem."""


PROBLEM_TYPES: Dict[str, Type[MaidanError]] = {
    f"{PROBLEM_BASE}not-found": NotFoundError,
    f"{PROBLEM_BASE}method-not-allowed": MethodNotAllowedError,
    f"{PROBLEM_BASE}conflict": ConflictError,
    f"{PROBLEM_BASE}bad-request": BadRequestError,
    f"{PROBLEM_BASE}unauthorized": UnauthorizedError,
    f"{PROBLEM_BASE}invalid-signature": InvalidSignatureError,
    f"{PROBLEM_BASE}forbidden": ForbiddenError,
    f"{PROBLEM_BASE}payload-too-large": PayloadTooLargeError,
    f"{PROBLEM_BASE}unsupported-media-type": UnsupportedMediaTypeError,
    f"{PROBLEM_BASE}rate-limited": RateLimitedError,
    f"{PROBLEM_BASE}bad-gateway": BadGatewayError,
    f"{PROBLEM_BASE}internal": InternalError,
    f"{PROBLEM_BASE}overloaded": OverloadedError,
    f"{PROBLEM_BASE}idempotency-key-reused": IdempotencyKeyReusedError,
    f"{PROBLEM_BASE}idempotency-key-in-flight": IdempotencyKeyInFlightError,
    f"{PROBLEM_BASE}cursor-too-old": CursorTooOldError,
    f"{PROBLEM_BASE}event-log-broken": EventLogBrokenError,
}


def problem_error(status: int, body: Any) -> MaidanError:
    """The error for a failed response: the class its problem ``type`` names."""
    type_ = body.get("type") if isinstance(body, dict) else None
    cls = PROBLEM_TYPES.get(type_, UnknownProblemError) if isinstance(type_, str) else UnknownProblemError
    return cls(status, body)
