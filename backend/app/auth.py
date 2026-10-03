"""Bearer identity and document-session ownership for the commercial API.

Every route except ``GET /api/health`` requires an ``Authorization: Bearer``
token listed in the ``PDFENGINE_API_KEYS`` environment variable. Each distinct
token is one tenant. The session store records only a SHA-256 subject id, so
the raw token is never kept next to the PDF.

A document id is not a capability. ``load_session`` returns a session only when
the caller is the subject that created it. A missing session, an expired
session, and a session owned by someone else all answer 404, so one tenant
cannot probe another's documents.

The table is process-local and bounded. ``PDFENGINE_MAX_UPLOAD_BYTES``
(default 32 MiB) caps one upload. ``PDFENGINE_MAX_SESSIONS`` (default 32) and
``PDFENGINE_MAX_RETAINED_BYTES`` (default 256 MiB) cap how much stays resident.
``PDFENGINE_SESSION_TTL_SECONDS`` (default 1800) is a sliding lifetime refreshed
on each successful load. A full table answers 429. Expired rows are dropped on
the next bind or load. A live session is never evicted to make room, including
a session that belongs to another tenant. ``byte_size`` is the accounted file
size used for that budget, not the process RSS.

A mutation holds that document's lock for the whole request. The lock is
acquired before the session table lock. Merging several documents locks their
ids in sorted order. A read does not take the lock.
"""

from __future__ import annotations

import hashlib
import hmac
import os
import secrets
import threading
import time
import uuid
from contextlib import contextmanager
from contextvars import ContextVar
from dataclasses import dataclass

from fastapi import HTTPException
from fastapi.responses import JSONResponse


MIN_TOKEN_LENGTH = 16
WS_TICKET_TTL_SECONDS = 60
MAX_WS_TICKETS = 1024
DEFAULT_MAX_UPLOAD_BYTES = 32 * 1024 * 1024
DEFAULT_SESSION_TTL_SECONDS = 1800
DEFAULT_MAX_SESSIONS = 32
DEFAULT_MAX_RETAINED_BYTES = 256 * 1024 * 1024

DOCUMENT_SESSIONS: dict[str, dict] = {}
_sessions_lock = threading.Lock()
_document_locks: dict[str, threading.Lock] = {}
_document_locks_guard = threading.Lock()

_current_subject: ContextVar["Subject | None"] = ContextVar("pdfengine_subject", default=None)
_ws_tickets: dict[str, tuple[str, float]] = {}


@dataclass(frozen=True)
class Subject:
    """Authenticated tenant. ``id`` is safe to store; it is not the bearer token."""

    id: str


def _configured_tokens() -> dict[str, str]:
    """Return ``token -> subject id`` for every well-formed configured key.

    Short tokens are ignored. An operator who sets ``PDFENGINE_API_KEYS`` to a
    guessable word does not accidentally open the API.
    """
    raw = os.environ.get("PDFENGINE_API_KEYS", "")
    tokens: dict[str, str] = {}
    for part in raw.split(","):
        token = part.strip()
        if len(token) < MIN_TOKEN_LENGTH:
            continue
        digest = hashlib.sha256(token.encode("utf-8")).hexdigest()
        tokens[token] = digest[:32]
    return tokens


def authenticate_header(authorization: str | None) -> Subject | None:
    """Resolve a bearer header to a subject, or ``None`` when it does not match.

    Comparison is on the SHA-256 digest via ``hmac.compare_digest``, so the
    check does not stop at the first differing character of the raw token.
    """
    if not authorization:
        return None
    scheme, _, credential = authorization.partition(" ")
    if scheme.lower() != "bearer" or not credential.strip():
        return None
    presented = hashlib.sha256(credential.strip().encode("utf-8")).digest()
    for token, subject_id in _configured_tokens().items():
        expected = hashlib.sha256(token.encode("utf-8")).digest()
        if hmac.compare_digest(presented, expected):
            return Subject(subject_id)
    return None


def current_subject() -> Subject:
    """Return the subject bound to this request. Raises 401 when there is none."""
    subject = _current_subject.get()
    if subject is None:
        raise HTTPException(status_code=401, detail="Authentication required.")
    return subject


def _positive_int_env(name: str, default: int) -> int:
    """Read a positive integer from the environment, or return ``default``.

    The value is read on each call so a test can change the limit without
    restarting the process. Zero, negative numbers, and non-integers fall back
    to the default rather than opening the table.
    """
    raw = os.environ.get(name, "")
    try:
        value = int(raw)
    except (TypeError, ValueError):
        return default
    if value <= 0:
        return default
    return value


def max_upload_bytes() -> int:
    """Largest accepted upload, in bytes."""
    return _positive_int_env("PDFENGINE_MAX_UPLOAD_BYTES", DEFAULT_MAX_UPLOAD_BYTES)


def session_ttl_seconds() -> int:
    """Sliding lifetime of a document session, in seconds."""
    return _positive_int_env("PDFENGINE_SESSION_TTL_SECONDS", DEFAULT_SESSION_TTL_SECONDS)


def max_sessions() -> int:
    """Maximum number of live document sessions in this process."""
    return _positive_int_env("PDFENGINE_MAX_SESSIONS", DEFAULT_MAX_SESSIONS)


def max_retained_bytes() -> int:
    """Maximum accounted bytes across live document sessions."""
    return _positive_int_env("PDFENGINE_MAX_RETAINED_BYTES", DEFAULT_MAX_RETAINED_BYTES)


def _document_lock(doc_id: str) -> threading.Lock:
    """Return the mutation lock for ``doc_id``, creating it if this is the first use."""
    with _document_locks_guard:
        lock = _document_locks.get(doc_id)
        if lock is None:
            lock = threading.Lock()
            _document_locks[doc_id] = lock
        return lock


def _drop_document_locks(doc_ids: list[str]) -> None:
    """Forget mutation locks for sessions that are already gone."""
    if not doc_ids:
        return
    with _document_locks_guard:
        for doc_id in doc_ids:
            _document_locks.pop(doc_id, None)


def clear_document_locks() -> None:
    """Drop every mutation lock. Tests use this together with the session table."""
    with _document_locks_guard:
        _document_locks.clear()


def _purge_expired_unlocked(now: float | None = None) -> list[str]:
    """Drop sessions whose ``expires_at`` is in the past. Caller holds the lock.

    The returned ids are forgotten from the mutation-lock table only after the
    session lock is released. This function does not take a document lock.
    """
    moment = time.time() if now is None else now
    expired = [
        key
        for key, session in DOCUMENT_SESSIONS.items()
        if float(session.get("expires_at", 0)) <= moment
    ]
    for key in expired:
        DOCUMENT_SESSIONS.pop(key, None)
    return expired


def _retained_bytes_unlocked() -> int:
    """Sum accounted bytes. Caller holds the lock."""
    return sum(int(session.get("byte_size", 0)) for session in DOCUMENT_SESSIONS.values())


def ensure_session_capacity(additional: int, additional_bytes: int = 0) -> None:
    """Reject when ``additional`` new sessions would exceed either cap.

    Expired sessions are removed first. This does not reserve the slots; the
    following ``bind_session`` checks again under the same lock.
    """
    if additional < 0 or additional_bytes < 0:
        raise HTTPException(status_code=429, detail="Document session capacity exceeded.")
    expired: list[str] = []
    try:
        with _sessions_lock:
            expired = _purge_expired_unlocked()
            if len(DOCUMENT_SESSIONS) + additional > max_sessions():
                raise HTTPException(status_code=429, detail="Document session capacity exceeded.")
            if _retained_bytes_unlocked() + additional_bytes > max_retained_bytes():
                raise HTTPException(status_code=429, detail="Document session capacity exceeded.")
    finally:
        _drop_document_locks(expired)


def bind_session(doc: object, filename: str, byte_size: int = 0) -> str:
    """Store a parsed document under a new id owned by the current subject.

    ``byte_size`` is the number of file bytes this session is charged. The
    original upload buffer is not stored again. The call fails closed when the
    session count or the retained-byte budget cannot fit this document.
    """
    subject = current_subject()
    charged = byte_size if byte_size > 0 else 0
    doc_id = str(uuid.uuid4())
    _document_lock(doc_id)
    expired: list[str] = []
    published = False
    try:
        with _sessions_lock:
            expired = _purge_expired_unlocked()
            if len(DOCUMENT_SESSIONS) + 1 > max_sessions():
                raise HTTPException(status_code=429, detail="Document session capacity exceeded.")
            if _retained_bytes_unlocked() + charged > max_retained_bytes():
                raise HTTPException(status_code=429, detail="Document session capacity exceeded.")
            now = time.time()
            DOCUMENT_SESSIONS[doc_id] = {
                "doc": doc,
                "filename": filename,
                "byte_size": charged,
                "subject_id": subject.id,
                "created_at": now,
                "expires_at": now + session_ttl_seconds(),
            }
            published = True
            return doc_id
    finally:
        if not published:
            expired.append(doc_id)
        _drop_document_locks(expired)


def set_session_byte_size(doc_id: str, byte_size: int) -> None:
    """Replace the accounted size of a session the caller owns.

    The previous charge is removed before the new one is tested, so replacing a
    document with a smaller file does not require extra budget. Growing past
    the retained-byte cap fails and leaves the previous charge in place.
    """
    subject = current_subject()
    charged = byte_size if byte_size > 0 else 0
    expired: list[str] = []
    try:
        with _sessions_lock:
            expired = _purge_expired_unlocked()
            session = DOCUMENT_SESSIONS.get(doc_id)
            if session is None or session.get("subject_id") != subject.id:
                raise HTTPException(status_code=404, detail="Document session not found.")
            others = _retained_bytes_unlocked() - int(session.get("byte_size", 0))
            if others + charged > max_retained_bytes():
                raise HTTPException(status_code=429, detail="Document session capacity exceeded.")
            session["byte_size"] = charged
            session["expires_at"] = time.time() + session_ttl_seconds()
    finally:
        _drop_document_locks(expired)


def load_session(doc_id: str) -> dict:
    """Return the session for ``doc_id`` when the current subject owns it.

    A successful load refreshes the sliding lifetime. Expired rows are removed
    first, so an expired id answers the same 404 as a missing id.
    """
    subject = current_subject()
    expired: list[str] = []
    try:
        with _sessions_lock:
            expired = _purge_expired_unlocked()
            session = DOCUMENT_SESSIONS.get(doc_id)
            if session is None or session.get("subject_id") != subject.id:
                raise HTTPException(status_code=404, detail="Document session not found.")
            session["expires_at"] = time.time() + session_ttl_seconds()
            return session
    finally:
        _drop_document_locks(expired)


def begin_document_mutation(doc_id: str) -> threading.Lock:
    """Acquire this document's mutation lock and confirm the session exists.

    The caller releases the returned lock. An async route runs this on a
    worker thread: acquiring on the event loop would stall every other request
    for as long as the document stays busy.
    """
    lock = _document_lock(doc_id)
    lock.acquire()
    try:
        load_session(doc_id)
    except BaseException:
        lock.release()
        raise
    return lock


@contextmanager
def document_mutation(doc_id: str):
    """Load ``doc_id`` while this request is the only mutation of that document.

    The lock is held until the caller finishes, including the time spent
    rewriting the file. ``load_session`` itself stays available to readers.
    """
    lock = begin_document_mutation(doc_id)
    try:
        yield load_session(doc_id)
    finally:
        lock.release()


@contextmanager
def document_mutations(doc_ids: list[str]):
    """Load every id while holding each document lock, lowest id first.

    The yielded sessions follow ``doc_ids``. Repeated ids share one lock.
    """
    locks = [_document_lock(doc_id) for doc_id in sorted(set(doc_ids))]
    for lock in locks:
        lock.acquire()
    try:
        yield [load_session(doc_id) for doc_id in doc_ids]
    finally:
        for lock in reversed(locks):
            lock.release()


def issue_ws_ticket() -> str:
    """Mint a single-use ticket so a browser WebSocket can prove the same subject.

    Browser WebSocket constructors cannot set an ``Authorization`` header.
    The HTTP caller exchanges its bearer token for a ticket that lives for
    ``WS_TICKET_TTL_SECONDS`` and is deleted on first use.
    """
    subject = current_subject()
    now = time.time()
    expired = [key for key, (_, expiry) in _ws_tickets.items() if expiry <= now]
    for key in expired:
        _ws_tickets.pop(key, None)
    if len(_ws_tickets) >= MAX_WS_TICKETS:
        raise HTTPException(status_code=429, detail="Too many pending WebSocket tickets.")
    ticket = secrets.token_urlsafe(32)
    _ws_tickets[ticket] = (subject.id, now + WS_TICKET_TTL_SECONDS)
    return ticket


def consume_ws_ticket(ticket: str) -> Subject | None:
    """Pop a live ticket and return its subject. Replays and expiry fail closed."""
    if not ticket:
        return None
    item = _ws_tickets.pop(ticket, None)
    if item is None:
        return None
    subject_id, expiry = item
    if expiry <= time.time():
        return None
    return Subject(subject_id)


def adopt_subject(subject: Subject):
    """Bind ``subject`` for the current task. The caller must reset the token."""
    return _current_subject.set(subject)


def reset_subject(token) -> None:
    """Restore the previous subject context."""
    _current_subject.reset(token)


class AuthMiddleware:
    """ASGI middleware. Runs in the request task so the subject contextvar is visible.

    ``BaseHTTPMiddleware`` executes the endpoint in a child task, which would
    drop the contextvar. This wrapper awaits the inner app directly.
    WebSocket routes authenticate themselves with a one-time ticket.
    """

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope.get("type") != "http":
            await self.app(scope, receive, send)
            return

        path = scope.get("path", "")
        if path == "/api/health" or scope.get("method") == "OPTIONS":
            await self.app(scope, receive, send)
            return

        headers = {
            key.decode("latin-1").lower(): value.decode("latin-1")
            for key, value in scope.get("headers", [])
        }
        subject = authenticate_header(headers.get("authorization"))
        if subject is None:
            response = JSONResponse(
                status_code=401,
                content={"detail": "Authentication required."},
            )
            await response(scope, receive, send)
            return

        token = adopt_subject(subject)
        try:
            await self.app(scope, receive, send)
        finally:
            reset_subject(token)
