"""Response models, from the server's OpenAPI schemas.

Each model is a dataclass built by :meth:`Model.from_dict`. Members the model
does not declare (added to the server after this client was published) are kept
in ``extra`` instead of failing the response. Timestamps are RFC 3339 strings.
A string enum field (``Thread.state``, ``Member.kind``, …) is a plain ``str``:
the values the server sends today are listed on the field, and a new value is
passed through rather than rejected.
"""

from __future__ import annotations

import dataclasses
import sys
import types
import typing
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Type, TypeVar

T = TypeVar("T", bound="Model")

# Wire names that are not Python identifiers.
_WIRE_NAMES = {"$type": "type"}


@dataclass(kw_only=True)
class Model:
    #: Members the server sent that this model does not declare.
    extra: Dict[str, Any] = field(default_factory=dict, repr=False, compare=False)

    @classmethod
    def from_dict(cls: Type[T], data: Any) -> T:
        if not isinstance(data, dict):
            raise ValueError(f"{cls.__name__}: expected a JSON object, got {type(data).__name__}")
        hints = _hints(cls)
        known = {f.name for f in dataclasses.fields(cls) if f.name != "extra"}
        kwargs: Dict[str, Any] = {}
        extra: Dict[str, Any] = {}
        for key, value in data.items():
            name = _WIRE_NAMES.get(key, key)
            if name in known:
                kwargs[name] = _convert(hints[name], value)
            else:
                extra[key] = value
        try:
            return cls(**kwargs, extra=extra)
        except TypeError as exc:
            raise ValueError(f"{cls.__name__}: {exc}") from None


_HINTS: Dict[type, Dict[str, Any]] = {}


def _hints(cls: type) -> Dict[str, Any]:
    if cls not in _HINTS:
        _HINTS[cls] = typing.get_type_hints(cls, vars(sys.modules[__name__]))
    return _HINTS[cls]


def _convert(hint: Any, value: Any) -> Any:
    if value is None:
        return None
    origin = typing.get_origin(hint)
    if origin in (typing.Union, types.UnionType):
        inner = [a for a in typing.get_args(hint) if a is not type(None)]
        return _convert(inner[0], value) if len(inner) == 1 else value
    if origin in (list, List) and isinstance(value, list):
        (item,) = typing.get_args(hint)
        return [_convert(item, v) for v in value]
    if isinstance(hint, type) and issubclass(hint, Model):
        return hint.from_dict(value)
    return value


def from_list(cls: Type[T], data: Any) -> List[T]:
    if not isinstance(data, list):
        raise ValueError(f"{cls.__name__}[]: expected a JSON array, got {type(data).__name__}")
    return [cls.from_dict(row) for row in data]


@dataclass(kw_only=True)
class Workspace(Model):
    id: str
    name: str
    created_at: str
    updated_at: str
    tombstoned_at: Optional[str] = None


@dataclass(kw_only=True)
class ImportResult(Model):
    workspace_id: str
    #: ``new`` or ``restore``.
    mode: str


@dataclass(kw_only=True)
class Member(Model):
    id: str
    workspace_id: str
    handle: str
    #: ``human`` or ``agent``.
    kind: str
    created_at: str
    updated_at: str
    display_name: Optional[str] = None
    tombstoned_at: Optional[str] = None


@dataclass(kw_only=True)
class TokenQuota(Model):
    capability: str
    max_per_window: int
    window_secs: int


@dataclass(kw_only=True)
class MintedToken(Model):
    """A mint's answer. ``secret`` is returned here once and never again."""

    id: str
    secret: str
    workspace_id: str
    member_id: str
    capabilities: List[str]
    quotas: List[TokenQuota]
    expires_at: Optional[str] = None


@dataclass(kw_only=True)
class TokenSummary(Model):
    """Token metadata; never carries the secret."""

    id: str
    workspace_id: str
    member_id: str
    capabilities: List[str]
    created_at: str
    label: Optional[str] = None
    expires_at: Optional[str] = None
    revoked_at: Optional[str] = None


@dataclass(kw_only=True)
class Channel(Model):
    id: str
    workspace_id: str
    name: str
    private: bool
    created_at: str
    updated_at: str
    topic: Optional[str] = None
    tombstoned_at: Optional[str] = None


@dataclass(kw_only=True)
class Thread(Model):
    id: str
    channel_id: str
    #: ``open``, ``in_review``, ``closed`` or ``archived``.
    state: str
    created_at: str
    updated_at: str
    parent_thread_id: Optional[str] = None
    title: Optional[str] = None
    assignee_id: Optional[str] = None
    owner_id: Optional[str] = None
    assignment_expires_at: Optional[str] = None
    #: The fencing token :meth:`Client.renew_claim` takes.
    claim_lease_id: Optional[str] = None
    work_started_at: Optional[str] = None
    tombstoned_at: Optional[str] = None


@dataclass(kw_only=True)
class StrongRef(Model):
    """A content-addressed pin (``maidan:event/{id}`` + its hash)."""

    uri: str
    content_hash: str


@dataclass(kw_only=True)
class ClaimedThread(Thread):
    """A claim: the thread's fields at the top level, plus the pin."""

    pin: StrongRef


@dataclass(kw_only=True)
class ThreadResult(Model):
    thread_id: str
    #: The producer's JSON, as it was set.
    result: Any
    produced_by: str
    produced_at: str


@dataclass(kw_only=True)
class Message(Model):
    id: str
    thread_id: str
    author_id: str
    body: str
    posted_at: str
    #: Structured blocks, each a dict discriminated by ``type`` (``text``,
    #: ``code``, ``tool_use``, ``tool_result``, ``resource_link``).
    content: Optional[List[Dict[str, Any]]] = None
    metadata: Any = None
    edited_at: Optional[str] = None
    tombstoned_at: Optional[str] = None


@dataclass(kw_only=True)
class Artifact(Model):
    id: str
    sha256: str
    size_bytes: int
    #: ``screenshot``, ``recording``, ``transcript``, ``code_dump``,
    #: ``attachment`` or ``context_snapshot``.
    kind: str
    created_at: str
    mime_type: Optional[str] = None
    uploaded_by: Optional[str] = None
    tombstoned_at: Optional[str] = None


@dataclass(kw_only=True)
class MessageEditView(Model):
    id: int
    message_id: str
    editor_id: str
    edited_at: str
    #: Present only with ``include_edits=true``.
    body_before: Optional[str] = None
    body_after: Optional[str] = None


@dataclass(kw_only=True)
class Reference(Model):
    id: str
    #: ``thread`` or ``message``.
    src_kind: str
    src_id: str
    dst_kind: str
    dst_id: str
    relation: str
    created_at: str


@dataclass(kw_only=True)
class ThreadTransition(Model):
    id: str
    thread_id: str
    from_state: str
    to_state: str
    actor_id: str
    occurred_at: str


@dataclass(kw_only=True)
class ThreadBrief(Model):
    """Stable thread fields. State and the lease are not here."""

    created_at: str
    title: Optional[str] = None
    parent_thread_id: Optional[str] = None
    owner_id: Optional[str] = None
    required_skills: List[str] = field(default_factory=list)


@dataclass(kw_only=True)
class AcceptedDecision(Model):
    thread_id: str
    state: str
    produced_by: str
    produced_at: str
    title: Optional[str] = None
    result_kind: Optional[str] = None
    status: Optional[str] = None
    summary: Optional[str] = None


@dataclass(kw_only=True)
class ThreadReview(Model):
    thread_id: str
    reviewer_id: str
    #: ``approve`` or ``request_changes``.
    decision: str
    created_at: str
    updated_at: str
    actor_id: Optional[str] = None
    note: Optional[str] = None
    dismissed_at: Optional[str] = None


@dataclass(kw_only=True)
class GlossaryTerm(Model):
    id: str
    workspace_id: str
    term: str
    definition: str
    aliases: List[str]
    created_by: str
    created_at: str
    updated_at: str


@dataclass(kw_only=True)
class PackElision(Model):
    elided_message_count: int
    elided_token_estimate: int
    first_elided_id: str
    last_elided_id: str
    summary: str


@dataclass(kw_only=True)
class ParentGrounding(Model):
    thread_id: str
    state: str
    title: Optional[str] = None
    opening_message: Optional[Message] = None
    latest_result: Any = None


@dataclass(kw_only=True)
class ThreadContext(Model):
    """``GET /threads/{id}/context``: the context pack a claimer reads."""

    workspace_id: str
    channel_id: str
    thread_id: str
    thread: ThreadBrief
    messages: List[Message]
    message_edits: List[MessageEditView]
    references: List[Reference]
    artifacts: List[Artifact]
    transitions: List[ThreadTransition]
    state: str
    updated_at: str
    prefix_sha256: str
    prefix_bytes: int
    glossary: List[GlossaryTerm] = field(default_factory=list)
    accepted_decisions: List[AcceptedDecision] = field(default_factory=list)
    change_requests: List[ThreadReview] = field(default_factory=list)
    assignee_id: Optional[str] = None
    assignment_expires_at: Optional[str] = None
    claim_lease_id: Optional[str] = None
    work_started_at: Optional[str] = None
    elision: Optional[PackElision] = None
    parent_grounding: Optional[ParentGrounding] = None
    as_of: Optional[int] = None
    next_message_cursor: Optional[str] = None


@dataclass(kw_only=True)
class StoredEvent(Model):
    """A row of ``GET /workspaces/{id}/events``. ``type`` is the wire's ``$type``;
    ``payload`` is the event itself, shaped by ``kind``."""

    type: str
    id: int
    lsn: int
    kind: str
    payload: Any
    occurred_at: str
    prev_hash: str
    content_hash: str
    workspace_id: Optional[str] = None
    channel_id: Optional[str] = None
    thread_id: Optional[str] = None
    content_key: Optional[str] = None
    traceparent: Optional[str] = None
    tracestate: Optional[str] = None


__all__ = [
    "AcceptedDecision",
    "Artifact",
    "Channel",
    "ClaimedThread",
    "GlossaryTerm",
    "ImportResult",
    "Member",
    "Message",
    "MessageEditView",
    "MintedToken",
    "Model",
    "PackElision",
    "ParentGrounding",
    "Reference",
    "StoredEvent",
    "StrongRef",
    "Thread",
    "ThreadContext",
    "ThreadBrief",
    "ThreadResult",
    "ThreadReview",
    "ThreadTransition",
    "TokenQuota",
    "TokenSummary",
    "Workspace",
]
