"""Bearer identity and document-session ownership for the commercial API.

Every route except ``GET /api/health`` requires an ``Authorization: Bearer``
token listed in the ``PDFENGINE_API_KEYS`` environment variable. Each distinct
token is one tenant. The session store records only a SHA-256 subject id, so
the raw token is never kept next to the PDF.

A document id is not a capability. ``load_session`` returns a session only when
the caller is the subject that created it. A missing session and a session
owned by someone else both answer 404, so one tenant cannot probe another's
documents.
"""

from __future__ import annotations

import hashlib
import hmac
import os
import secrets
import time
import uuid
from contextvars import ContextVar
from dataclasses import dataclass

from fastapi import HTTPException
from fastapi.responses import JSONResponse


MIN_TOKEN_LENGTH = 16
WS_TICKET_TTL_SECONDS = 60
MAX_WS_TICKETS = 1024

DOCUMENT_SESSIONS: dict[str, dict] = {}

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


def bind_session(doc: object, filename: str, raw_bytes: bytes | None = None) -> str:
    """Store a parsed document under a new id owned by the current subject."""
    subject = current_subject()
    doc_id = str(uuid.uuid4())
    DOCUMENT_SESSIONS[doc_id] = {
        "doc": doc,
        "filename": filename,
        "raw_bytes": raw_bytes if raw_bytes is not None else b"",
        "subject_id": subject.id,
        "created_at": time.time(),
    }
    return doc_id


def load_session(doc_id: str) -> dict:
    """Return the session for ``doc_id`` when the current subject owns it."""
    subject = current_subject()
    session = DOCUMENT_SESSIONS.get(doc_id)
    if session is None or session.get("subject_id") != subject.id:
        raise HTTPException(status_code=404, detail="Document session not found.")
    return session


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
