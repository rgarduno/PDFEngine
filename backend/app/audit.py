"""Append-only record of who uploaded, redacted, signed, or optimized a document.

Each event stores an action name, the subject id, the document id, and a UTC
timestamp. It does not store PDF bytes, filenames, passwords, tokens, or the
text that a redaction removed. The buffer is bounded. When it is full, the
oldest event is dropped.
"""

from __future__ import annotations

import logging
import threading
from collections import deque
from dataclasses import dataclass
from datetime import datetime, timezone

from app.auth import current_subject


MAX_AUDIT_EVENTS = 4096
_MAX_ID_LENGTH = 64
_ACTIONS = frozenset({"upload", "redact", "sign", "optimize", "metadata_updated"})

logger = logging.getLogger("pdfengine.audit")

_events: deque["AuditEvent"] = deque(maxlen=MAX_AUDIT_EVENTS)
_lock = threading.Lock()


@dataclass(frozen=True)
class AuditEvent:
    """One recorded action. The fields are safe to show back to that subject."""

    action: str
    subject_id: str
    document_id: str
    at: str


def _token(value: object) -> bool:
    """True when ``value`` is a single-line id made of ASCII letters, digits, or hyphens."""
    if not isinstance(value, str) or not value or len(value) > _MAX_ID_LENGTH:
        return False
    return all(ch.isascii() and (ch.isalnum() or ch == "-") for ch in value)


def record(action: str, subject_id: str, document_id: str) -> bool:
    """Append one event. Returns ``False`` when the action or an id is not accepted.

    A rejected call stores nothing. Callers cannot use this function to write a
    password, a bearer token, or a free-form note into the log.
    """
    if action not in _ACTIONS or not _token(subject_id) or not _token(document_id):
        return False
    event = AuditEvent(
        action=action,
        subject_id=subject_id,
        document_id=document_id,
        at=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    )
    with _lock:
        _events.append(event)
    logger.info(
        "action=%s subject=%s document=%s at=%s",
        event.action,
        event.subject_id,
        event.document_id,
        event.at,
    )
    return True


def record_document_action(action: str, document_id: str) -> None:
    """Record ``action`` for the subject bound to this request."""
    record(action, current_subject().id, document_id)


def events_for(subject_id: str) -> list[AuditEvent]:
    """Return a snapshot of events owned by ``subject_id``, oldest first."""
    with _lock:
        return [event for event in _events if event.subject_id == subject_id]


def clear_events() -> None:
    """Drop every event. Tests use this so one case cannot see another."""
    with _lock:
        _events.clear()
