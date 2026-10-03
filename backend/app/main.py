"""PDFEngine Commercial REST API.

High-performance FastAPI service providing document ingestion,
interactive scene graph layout inspection, and surgical in-place PDF editing.
"""

import asyncio
import base64
import inspect
import logging
import os
from functools import wraps
from typing import List, Optional

from fastapi import FastAPI, File, Form, HTTPException, UploadFile, WebSocket, WebSocketDisconnect
from fastapi.responses import Response

from app.audit import events_for, record_document_action
from app import tsa
from app.auth import (
    AuthMiddleware,
    adopt_subject,
    begin_document_mutation,
    bind_session,
    consume_ws_ticket,
    current_subject,
    document_mutation,
    document_mutations,
    ensure_session_capacity,
    issue_ws_ticket,
    load_session,
    max_upload_bytes,
    reset_subject,
    set_session_byte_size,
)

try:
    import pdf_engine
except ImportError:
    pdf_engine = None

from app.models import (
    AddLinkRequest,
    AddMarkupRequest,
    AddPaginationRequest,
    AddStampRequest,
    AddTextWatermarkRequest,
    AnnotationActionResponse,
    AnnotationModel,
    AuditEventResponse,
    BatchFillFormsRequest,
    BoundingBox,
    CreateFormFieldRequest,
    CreateFormFieldResponse,
    DeleteFormFieldResponse,
    DeletePagesRequest,
    DocumentFormsResponse,
    DocumentOverviewResponse,
    DocumentUploadResponse,
    EditParagraphRequest,
    EditParagraphResponse,
    FillFormsResponse,
    FlattenAnnotationsResponse,
    FlattenFormsResponse,
    FormFieldModel,
    UpdateFormFieldRequest,
    UpdateFormFieldResponse,
    ImageModel,
    MergeDocumentsRequest,
    MergeDocumentsResponse,
    PageAnnotationsResponse,
    PageImagesResponse,
    PageOperationResponse,
    PageOverviewItem,
    PageSceneGraph,
    ParagraphModel,
    ReorderPagesRequest,
    RotatePageRequest,
    RotatePageResponse,
    SplitDocumentRequest,
    SplitDocumentResponse,
    WatermarkActionResponse,
    RedactionRegionItem,
    RedactRegionsRequest,
    RedactPatternRequest,
    RedactTextRequest,
    SanitizeDocumentRequest,
    RedactionSummaryModel,
    RedactionActionResponse,
    SanitizeDocumentResponse,
    PermissionsModel,
    EncryptDocumentRequest,
    DecryptDocumentRequest,
    SignDocumentRequest,
    SignatureModel,
    SecurityStatusResponse,
    SecurityActionResponse,
    TableCellModel,
    TableModel,
    PageTablesResponse,
    TableExportResponse,
    OptimizeRequest,
    OptimizeResponse,
)

app = FastAPI(
    title="PDFEngine API",
    description="High-performance, lossless PDF parsing and surgical editing service",
    version="0.1.0",
)

logger = logging.getLogger("pdfengine.api")

# Shown to clients when the engine raises. The cause is written to the server log.
_PUBLIC_FAILURE_DETAIL = "The request could not be completed."
_CORS_ALLOW_METHODS = b"GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD"


def serialized_mutation(func):
    """Serialize calls that mutate the document named by ``doc_id``.

    The lock covers the whole call. Readers keep using ``load_session`` and
    do not pass through here. Merge locks its sources itself, in id order.
    """
    signature = inspect.signature(func)

    def doc_id_of(args, kwargs) -> str:
        bound = signature.bind_partial(*args, **kwargs)
        doc_id = bound.arguments.get("doc_id")
        if not isinstance(doc_id, str) or not doc_id:
            raise HTTPException(status_code=404, detail="Document session not found.")
        return doc_id

    if inspect.iscoroutinefunction(func):

        @wraps(func)
        async def wrapper(*args, **kwargs):
            lock = await asyncio.to_thread(
                begin_document_mutation, doc_id_of(args, kwargs)
            )
            try:
                return await func(*args, **kwargs)
            finally:
                lock.release()

        return wrapper

    @wraps(func)
    def wrapper(*args, **kwargs):
        with document_mutation(doc_id_of(args, kwargs)):
            return func(*args, **kwargs)

    return wrapper


def _public_error(status_code: int, exc: BaseException) -> HTTPException:
    """Returns a stable client error and records the engine failure on the server."""
    logger.warning("Request failed (%s)", type(exc).__name__, exc_info=exc)
    return HTTPException(status_code=status_code, detail=_PUBLIC_FAILURE_DETAIL)


def _cors_origins() -> list[str]:
    """Exact browser origins allowed to call the API.

    ``PDFENGINE_CORS_ORIGINS`` is a comma-separated list. A wildcard is ignored.
    An empty list allows no browser origin, and credentials are never sent with one.
    """
    raw = os.environ.get("PDFENGINE_CORS_ORIGINS", "")
    origins: list[str] = []
    for part in raw.split(","):
        item = part.strip()
        if not item or item == "*" or not item.isascii():
            continue
        if any(ch in item for ch in "\r\n\x00"):
            continue
        if not (item.startswith("http://") or item.startswith("https://")):
            continue
        origins.append(item)
    return origins


class ExplicitOriginMiddleware:
    """Reflects ``Origin`` only when that exact value is configured.

    Registered outside authentication so a preflight is answered before the
    bearer check. Credentials are attached only to a reflected origin.
    """

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return

        header_map = {
            key.decode("latin-1").lower(): value.decode("latin-1")
            for key, value in scope.get("headers", [])
        }
        origin = header_map.get("origin")
        reflect = origin if origin and origin in _cors_origins() else None

        if (
            scope["method"] == "OPTIONS"
            and origin
            and "access-control-request-method" in header_map
        ):
            await self._preflight(send, reflect, header_map)
            return

        async def send_cors(message):
            if message["type"] == "http.response.start":
                headers = list(message.get("headers") or [])
                headers.append((b"vary", b"Origin"))
                if reflect is not None:
                    headers.append((b"access-control-allow-origin", reflect.encode("ascii")))
                    headers.append((b"access-control-allow-credentials", b"true"))
                message = {**message, "headers": headers}
            await send(message)

        await self.app(scope, receive, send_cors)

    async def _preflight(self, send, reflect: Optional[str], header_map: dict) -> None:
        headers = [(b"vary", b"Origin")]
        status = 400
        if reflect is not None:
            status = 204
            headers.extend(
                [
                    (b"access-control-allow-origin", reflect.encode("ascii")),
                    (b"access-control-allow-credentials", b"true"),
                    (b"access-control-allow-methods", _CORS_ALLOW_METHODS),
                    (b"access-control-max-age", b"600"),
                ]
            )
            requested = header_map.get("access-control-request-headers", "").strip()
            if requested and "\r" not in requested and "\n" not in requested:
                headers.append(
                    (b"access-control-allow-headers", requested.encode("latin-1"))
                )
        await send({"type": "http.response.start", "status": status, "headers": headers})
        await send({"type": "http.response.body", "body": b""})


# Auth is registered first so the origin middleware stays outermost.
# A preflight is answered before the bearer check, and a 401 still carries
# the origin headers when the caller is on the configured list.
app.add_middleware(AuthMiddleware)
app.add_middleware(ExplicitOriginMiddleware)


@app.get("/api/health")
def health_check():
    """Health check endpoint for container orchestrators and load balancers."""
    return {
        "status": "healthy",
        "engine_loaded": pdf_engine is not None,
        "version": "0.1.0",
    }


@app.get("/api/audit", response_model=List[AuditEventResponse])
def list_audit_events():
    """Returns the caller's action log.

    Each row is an upload, redaction, signature attestation, or optimization.
    Rows belonging to another subject are not included.
    """
    subject = current_subject()
    return [
        AuditEventResponse(
            action=event.action,
            document_id=event.document_id,
            at=event.at,
        )
        for event in events_for(subject.id)
    ]


@app.post("/api/auth/ws-ticket")
def create_ws_ticket():
    """Exchange the caller's bearer token for a single-use WebSocket ticket."""
    return {"ticket": issue_ws_ticket(), "expires_in": 60}


_UPLOAD_CHUNK_BYTES = 1024 * 1024


async def _read_bounded_upload(file: UploadFile) -> bytes:
    """Read an upload in chunks and stop once it passes the configured cap.

    At most one extra byte past the limit is read, so a client cannot make the
    process buffer an entire oversized body before the request is rejected.
    """
    limit = max_upload_bytes()
    chunks: list[bytes] = []
    total = 0
    while True:
        remaining = limit - total + 1
        if remaining <= 0:
            raise HTTPException(status_code=413, detail="Upload exceeds the configured size limit.")
        chunk = await file.read(min(_UPLOAD_CHUNK_BYTES, remaining))
        if not chunk:
            break
        total += len(chunk)
        if total > limit:
            raise HTTPException(status_code=413, detail="Upload exceeds the configured size limit.")
        chunks.append(chunk)
    return b"".join(chunks)


@app.post("/api/documents/upload", response_model=DocumentUploadResponse)
async def upload_document(file: UploadFile = File(...)):
    """Uploads a PDF document, indexes pages, and initializes an editing session."""
    if not file.filename.lower().endswith(".pdf"):
        raise HTTPException(status_code=400, detail="Only PDF documents are supported.")

    content = await _read_bounded_upload(file)
    if len(content) < 8 or not content.startswith(b"%PDF-"):
        raise HTTPException(status_code=400, detail="Uploaded file is not a valid PDF.")

    if pdf_engine is None:
        raise HTTPException(
            status_code=500, detail="Native pdf_engine core extension is not loaded."
        )

    try:
        doc = pdf_engine.Document.from_bytes(content)
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(422, e)

    page_count = doc.page_count()
    doc_id = bind_session(doc, file.filename, len(content))
    record_document_action("upload", doc_id)

    return DocumentUploadResponse(
        document_id=doc_id,
        filename=file.filename,
        page_count=page_count,
    )


@app.get("/api/documents/{doc_id}/pages/{page_idx}/scenegraph", response_model=PageSceneGraph)
def get_page_scenegraph(doc_id: str, page_idx: int):
    """Retrieves the layout scene graph (paragraphs, bounding boxes, alignments) for a page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        page = doc.get_page(page_idx)
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)

    paragraphs_raw = page.get_paragraphs()
    paragraph_models = []

    for p in paragraphs_raw:
        min_x, min_y, max_x, max_y = p.bbox()
        paragraph_models.append(
            ParagraphModel(
                id=p.id,
                text=p.text,
                bbox=BoundingBox(
                    min_x=round(min_x, 2),
                    min_y=round(min_y, 2),
                    max_x=round(max_x, 2),
                    max_y=round(max_y, 2),
                    width=round(max_x - min_x, 2),
                    height=round(max_y - min_y, 2),
                ),
                alignment=p.alignment,
                leading=round(p.leading, 2),
                line_count=p.line_count,
            )
        )

    return PageSceneGraph(
        page_number=page_idx,
        paragraphs=paragraph_models,
    )


_OVERVIEW_DEFAULT_LIMIT = 24
_OVERVIEW_MAX_LIMIT = 48


@app.get("/api/documents/{doc_id}/pages/overview", response_model=DocumentOverviewResponse)
def get_document_overview(doc_id: str, offset: int = 0, limit: int = _OVERVIEW_DEFAULT_LIMIT):
    """Returns one window of page previews.

    Each call reads at most 48 pages. `offset` is the first 0-based page
    index in the window. Clients request the next window explicitly.
    """
    session = load_session(doc_id)

    if offset < 0:
        offset = 0
    if limit < 1:
        limit = _OVERVIEW_DEFAULT_LIMIT
    if limit > _OVERVIEW_MAX_LIMIT:
        limit = _OVERVIEW_MAX_LIMIT

    doc = session["doc"]
    filename = session.get("filename", "document.pdf")
    total_pages = doc.page_count()
    start = min(offset, total_pages)
    end = min(total_pages, start + limit)
    pages_overview = []

    for p_num in range(start + 1, end + 1):
        try:
            rotation = doc.get_page_rotation(p_num)
        except Exception:
            rotation = 0

        try:
            page = doc.get_page(p_num)
            paras = page.get_paragraphs()
            para_count = len(paras)
            preview_snippet = paras[0].text[:80].strip() if paras else ""
        except Exception:
            para_count = 0
            preview_snippet = ""

        pages_overview.append(
            PageOverviewItem(
                page_number=p_num,
                page_index=p_num - 1,
                rotation=rotation,
                paragraph_count=para_count,
                preview_snippet=preview_snippet,
            )
        )

    return DocumentOverviewResponse(
        document_id=doc_id,
        filename=filename,
        total_pages=total_pages,
        offset=start,
        limit=limit,
        pages=pages_overview,
    )


@app.get("/api/documents/{doc_id}/pages/{page_idx}/rotation")
def get_page_rotation_endpoint(doc_id: str, page_idx: int):
    """Gets the current rotation degrees for a specific page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        rotation = doc.get_page_rotation(page_idx)
        return {"document_id": doc_id, "page_number": page_idx, "rotation": rotation}
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)



@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/edit/{paragraph_id}",
    response_model=EditParagraphResponse,
)
@serialized_mutation
def edit_paragraph(
    doc_id: str, page_idx: int, paragraph_id: int, request: EditParagraphRequest
):
    """Performs an in-place surgical paragraph edit, recalculating reflow without altering graphics."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        page = doc.get_page(page_idx)
        page.edit_paragraph(paragraph_id, request.new_text)
        doc.update_page(page)
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)

    return EditParagraphResponse(
        success=True,
        document_id=doc_id,
        page_number=page_idx,
        paragraph_id=paragraph_id,
        updated_text=request.new_text,
        message="Paragraph surgically replaced with zero layout drift.",
    )


def _content_disposition(kind: str, filename: str) -> str:
    """Builds a Content-Disposition value from a server-chosen ASCII name.

    A name that is not a plain token is replaced so a client filename cannot
    break out of the header.
    """
    safe = filename if (
        filename.isascii()
        and filename
        and all(ch.isalnum() or ch in "._-" for ch in filename)
    ) else "document.pdf"
    return f'{kind}; filename="{safe}"'


def _download_token(raw: str, fallback: str) -> str:
    """Keeps letters, digits, hyphen, and underscore from a caller-supplied label."""
    token = "".join(ch for ch in raw if ch.isascii() and (ch.isalnum() or ch in "-_"))
    return token or fallback


def _link_scheme_refusal(exc: Exception) -> Optional[HTTPException]:
    """Maps a rejected link scheme to a stable client error."""
    if "Link URI must use the http or https scheme." in str(exc):
        return HTTPException(
            status_code=400,
            detail="Only http and https links are accepted.",
        )
    return None


_SIGNING_DETAILS = (
    "Certificate and private key are required.",
    "Provide either PEM credentials or a PKCS#12 file.",
    "PKCS#12 password is required.",
    "PKCS#12 could not be opened.",
    "PKCS#12 bag uses an unsupported cipher.",
    "PKCS#12 contains more than one private key.",
    "PKCS#12 does not contain a private key.",
    "Certificate or private key could not be read.",
    "Signing material is too large.",
    "CMS does not fit the reserved contents hole.",
    "Timestamp token was rejected.",
    "A timestamp is already present.",
    "Unsupported certificate public key.",
    "Private key does not match the certificate.",
    "Signature could not be verified.",
    "A certificate and private key are required to request a timestamp.",
    "Timestamp authority URL was rejected.",
    "Timestamp authority request failed.",
)
_PEM_CAP = 64 * 1024
_PKCS12_TEXT_CAP = 100_000
_PKCS12_DER_CAP = 96 * 1024
_PASSWORD_CAP = 256
_TSA_URL_CAP = 512


def _signing_failure(exc: BaseException) -> HTTPException:
    """Maps a signing failure to a stable sentence. Unknown text stays in the log."""
    text = str(exc)
    for sentence in _SIGNING_DETAILS:
        if sentence in text:
            return HTTPException(status_code=400, detail=sentence)
    return _public_error(400, exc)


def _timestamp_credentials(request: SignDocumentRequest) -> bool:
    pem = request.certificate_pem is not None and request.private_key_pem is not None
    return pem or request.pkcs12_base64 is not None


def _uses_signing_material(request: SignDocumentRequest) -> bool:
    return any(
        value is not None
        for value in (
            request.certificate_pem,
            request.private_key_pem,
            request.chain_pem,
            request.pkcs12_base64,
            request.pkcs12_password,
        )
    )


def _prepare_signing_material(request: SignDocumentRequest) -> Optional[bytes]:
    """Validates credential shape. Returns PKCS#12 DER, or None for PEM or an attestation."""
    if request.tsa_url is not None and not _timestamp_credentials(request):
        raise HTTPException(
            status_code=400,
            detail="A certificate and private key are required to request a timestamp.",
        )
    if request.tsa_url is not None:
        try:
            tsa.validate_tsa_url(request.tsa_url)
        except ValueError:
            raise HTTPException(
                status_code=400,
                detail="Timestamp authority URL was rejected.",
            )
    for value in (request.certificate_pem, request.private_key_pem, request.chain_pem):
        if value is not None and len(value) > _PEM_CAP:
            raise HTTPException(status_code=400, detail="Signing material is too large.")
    if request.pkcs12_password is not None and len(request.pkcs12_password.encode("utf-8")) > _PASSWORD_CAP:
        raise HTTPException(status_code=400, detail="Signing material is too large.")
    if request.tsa_url is not None and len(request.tsa_url) > _TSA_URL_CAP:
        raise HTTPException(status_code=400, detail="Signing material is too large.")
    if request.pkcs12_base64 is not None and len(request.pkcs12_base64) > _PKCS12_TEXT_CAP:
        raise HTTPException(status_code=400, detail="Signing material is too large.")

    pem_side = any(
        value is not None
        for value in (request.certificate_pem, request.private_key_pem, request.chain_pem)
    )
    p12_side = request.pkcs12_base64 is not None or request.pkcs12_password is not None
    if pem_side and p12_side:
        raise HTTPException(
            status_code=400,
            detail="Provide either PEM credentials or a PKCS#12 file.",
        )
    if request.pkcs12_password is not None and request.pkcs12_base64 is None:
        raise HTTPException(
            status_code=400,
            detail="Provide either PEM credentials or a PKCS#12 file.",
        )
    if pem_side:
        if not request.certificate_pem or not request.private_key_pem:
            raise HTTPException(
                status_code=400,
                detail="Certificate and private key are required.",
            )
        return None
    if request.pkcs12_base64 is None:
        return None
    if request.pkcs12_password is None:
        raise HTTPException(status_code=400, detail="PKCS#12 password is required.")
    try:
        der = base64.b64decode(request.pkcs12_base64, validate=True)
    except Exception:
        raise HTTPException(status_code=400, detail="PKCS#12 could not be opened.")
    if not der:
        raise HTTPException(status_code=400, detail="PKCS#12 could not be opened.")
    if len(der) > _PKCS12_DER_CAP:
        raise HTTPException(status_code=400, detail="Signing material is too large.")
    return der


def _rollback_document(doc, snapshot: Optional[bytes]) -> None:
    if snapshot is not None:
        doc.replace_bytes(snapshot)


def _optimization_refusal(exc: Exception) -> Optional[HTTPException]:
    """Maps the engine's protected-file refusal to a stable client error.

    The rewrite is not attempted. The detail does not include the engine traceback.
    """
    if "Optimization refused" in str(exc):
        return HTTPException(
            status_code=409,
            detail="Optimization is refused for encrypted or signed documents.",
        )
    return None


def _download_bytes(doc):
    """Returns stored bytes while a PKCS#7 signature still verifies.

    An attestation is rewritten so its digest is sealed again. A full rewrite
    would move a PKCS#7 byte range, so that download returns the stored file.
    Later edits stay out of that download until the signature no longer verifies.
    """
    try:
        signatures = doc.verify_signatures()
    except Exception:
        signatures = ()
    for sig in signatures:
        if sig.sub_filter == "adbe.pkcs7.detached" and sig.byte_range_valid:
            return doc.raw_bytes()
    return doc.save_to_bytes()


@app.get("/api/documents/{doc_id}/export")
def export_document(doc_id: str, optimized: bool = False):
    """Serializes the edited document into a downloadable PDF binary.

    A still-valid detached PKCS#7 signature is downloaded as stored. An
    attestation is rewritten and resealed.
    """
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        if optimized and "optimized_bytes" in session:
            pdf_bytes = session["optimized_bytes"]
        elif optimized:
            pdf_bytes, _ = doc.save_optimized_to_bytes()
        else:
            pdf_bytes = _download_bytes(doc)
    except HTTPException:
        raise
    except Exception as e:
        refusal = _optimization_refusal(e)
        if refusal is not None:
            raise refusal
        raise _public_error(500, e)

    export_name = "document_optimized.pdf" if optimized else "document_edited.pdf"

    return Response(
        content=bytes(pdf_bytes),
        media_type="application/pdf",
        headers={"Content-Disposition": _content_disposition("attachment", export_name)},
    )


@app.get("/api/documents/{doc_id}/pages/{page_idx}/fonts")
def list_page_fonts(doc_id: str, page_idx: int):
    """Lists all font resource identifiers declared on a specific page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        fonts = doc.get_page_fonts(page_idx)
        return {
            "page_number": page_idx,
            "fonts": list(fonts.keys()),
            "embedded_count": len(fonts),
        }
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.get("/api/documents/{doc_id}/pages/{page_idx}/fonts/{font_name}")
def get_page_font_binary(doc_id: str, page_idx: int, font_name: str):
    """Extracts and streams embedded TrueType/OpenType font binaries for dynamic browser @font-face registration."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        fonts = doc.get_page_fonts(page_idx)
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)

    font_bytes = fonts.get(font_name)
    if not font_bytes:
        clean_name = font_name.lstrip("/")
        font_bytes = fonts.get(clean_name)

    if not font_bytes:
        raise HTTPException(
            status_code=404,
            detail=f"Font '{font_name}' has no embedded binary on page {page_idx}.",
        )

    font_token = _download_token(font_name.lstrip("/"), "embedded")
    return Response(
        content=bytes(font_bytes),
        media_type="font/ttf",
        headers={
            "Content-Disposition": _content_disposition("inline", f"{font_token}.ttf"),
            "Cache-Control": "public, max-age=86400",
        },
    )


@app.get(
    "/api/documents/{doc_id}/pages/{page_idx}/images",
    response_model=PageImagesResponse,
)
def get_page_images(doc_id: str, page_idx: int):
    """Lists all Image XObjects declared and placed on a specific page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        raw_images = doc.get_page_images(page_idx)
        images = []
        for img in raw_images:
            images.append(
                ImageModel(
                    id=img.id,
                    name=img.name,
                    width_px=img.width_px,
                    height_px=img.height_px,
                    color_space=img.color_space,
                    bits_per_component=img.bits_per_component,
                    filter=img.filter,
                    byte_size=img.byte_size,
                    bbox=BoundingBox(
                        min_x=img.min_x,
                        min_y=img.min_y,
                        max_x=img.max_x,
                        max_y=img.max_y,
                        width=img.max_x - img.min_x,
                        height=img.max_y - img.min_y,
                    ),
                )
            )
        return PageImagesResponse(
            page_number=page_idx,
            images=images,
            count=len(images),
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.get("/api/documents/{doc_id}/images/{image_id}")
def get_image_binary(doc_id: str, image_id: int):
    """Extracts and streams raw image binary (JPEG or PNG) for browser display."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        content_bytes, mime_type = doc.get_image_binary(image_id)
        ext = "jpg" if "jpeg" in mime_type else "png"
        return Response(
            content=bytes(content_bytes),
            media_type=mime_type,
            headers={
                "Content-Disposition": f'inline; filename="image_{image_id}.{ext}"',
                "Cache-Control": "public, max-age=86400",
            },
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(404, e)


@app.post("/api/documents/{doc_id}/images/{image_id}/replace")
@serialized_mutation
async def replace_image(doc_id: str, image_id: int, file: UploadFile = File(...)):
    """Surgically replaces an existing image XObject in the PDF with a new JPEG or PNG file."""
    session = load_session(doc_id)

    new_bytes = await file.read()
    if not new_bytes:
        raise HTTPException(status_code=400, detail="Uploaded image file is empty.")

    doc = session["doc"]
    try:
        doc.replace_image(image_id, new_bytes)
        return {
            "success": True,
            "document_id": doc_id,
            "image_id": image_id,
            "filename": file.filename,
            "byte_size": len(new_bytes),
            "message": f"Image {image_id} successfully replaced in-place with {file.filename}.",
        }
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.get("/api/documents/{doc_id}/forms", response_model=DocumentFormsResponse)
def get_document_forms(doc_id: str):
    """Retrieves all interactive AcroForm fields present in the document."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        raw_fields = doc.get_form_fields()
        field_models = []
        for f in raw_fields:
            min_x, min_y, max_x, max_y = f.bbox()
            field_models.append(
                FormFieldModel(
                    id=f.id,
                    name=f.name,
                    alt_name=f.alt_name,
                    field_type=f.field_type,
                    value=f.value,
                    default_value=f.default_value,
                    bbox=BoundingBox(
                        min_x=round(min_x, 2),
                        min_y=round(min_y, 2),
                        max_x=round(max_x, 2),
                        max_y=round(max_y, 2),
                        width=round(max_x - min_x, 2),
                        height=round(max_y - min_y, 2),
                    ),
                    page_number=f.page_number,
                    options=f.options,
                    is_read_only=f.is_read_only,
                    is_required=f.is_required,
                    is_multiline=f.is_multiline,
                    max_length=f.max_length,
                )
            )
        return DocumentFormsResponse(
            document_id=doc_id,
            count=len(field_models),
            fields=field_models,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/forms/fill", response_model=FillFormsResponse)
@serialized_mutation
def fill_document_forms(doc_id: str, request: BatchFillFormsRequest):
    """Fills one or more interactive form fields by name in a batch transaction."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        updated_count = doc.fill_form_fields(request.fields)
        return FillFormsResponse(
            success=True,
            document_id=doc_id,
            updated_count=updated_count,
            message=f"Successfully filled {updated_count} form field(s).",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/forms/flatten", response_model=FlattenFormsResponse)
@serialized_mutation
def flatten_document_forms_endpoint(doc_id: str):
    """Permanently flattens all interactive form fields into page vectors and strips widget annotations."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        flattened_count = doc.flatten_forms()
        return FlattenFormsResponse(
            success=True,
            document_id=doc_id,
            flattened_count=flattened_count,
            message=f"Successfully flattened {flattened_count} form field(s) into permanent page graphics.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/pages/{page_idx}/forms", response_model=CreateFormFieldResponse)
@serialized_mutation
def create_form_field_endpoint(doc_id: str, page_idx: int, request: CreateFormFieldRequest):
    """Creates and places a new interactive AcroForm field on a specific page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        f = doc.add_form_field(
            page_number=page_idx,
            name=request.name,
            field_type=request.field_type,
            min_x=request.min_x,
            min_y=request.min_y,
            max_x=request.max_x,
            max_y=request.max_y,
            value=request.value,
            default_value=request.default_value,
            alt_name=request.alt_name,
            options=request.options,
            is_read_only=request.is_read_only,
            is_required=request.is_required,
            is_multiline=request.is_multiline,
            max_length=request.max_length,
            font_size=request.font_size,
        )
        min_x, min_y, max_x, max_y = f.bbox()
        field_model = FormFieldModel(
            id=f.id,
            name=f.name,
            alt_name=f.alt_name,
            field_type=f.field_type,
            value=f.value,
            default_value=f.default_value,
            bbox=BoundingBox(
                min_x=round(min_x, 2),
                min_y=round(min_y, 2),
                max_x=round(max_x, 2),
                max_y=round(max_y, 2),
                width=round(max_x - min_x, 2),
                height=round(max_y - min_y, 2),
            ),
            page_number=f.page_number,
            options=f.options,
            is_read_only=f.is_read_only,
            is_required=f.is_required,
            is_multiline=f.is_multiline,
            max_length=f.max_length,
        )
        return CreateFormFieldResponse(
            status="ok",
            document_id=doc_id,
            field=field_model,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.delete("/api/documents/{doc_id}/forms/{field_name}", response_model=DeleteFormFieldResponse)
@serialized_mutation
def delete_form_field_endpoint(doc_id: str, field_name: str):
    """Deletes an interactive form field from the document by name."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        deleted = doc.delete_form_field(field_name)
        if not deleted:
            raise HTTPException(status_code=404, detail=f"Form field '{field_name}' not found.")
        return DeleteFormFieldResponse(
            status="ok",
            document_id=doc_id,
            deleted=True,
            field_name=field_name,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.put("/api/documents/{doc_id}/forms/{field_name}", response_model=UpdateFormFieldResponse)
@serialized_mutation
def update_form_field_endpoint(doc_id: str, field_name: str, request: UpdateFormFieldRequest):
    """Updates geometry or properties of an existing form field."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        updated = doc.update_form_field(
            name=field_name,
            min_x=request.min_x,
            min_y=request.min_y,
            max_x=request.max_x,
            max_y=request.max_y,
            alt_name=request.alt_name,
            is_read_only=request.is_read_only,
            is_required=request.is_required,
            is_multiline=request.is_multiline,
        )
        if not updated:
            raise HTTPException(status_code=404, detail=f"Form field '{field_name}' not found.")

        min_x, min_y, max_x, max_y = updated.bbox()
        field_model = FormFieldModel(
            id=updated.id,
            name=updated.name,
            alt_name=updated.alt_name,
            field_type=updated.field_type,
            value=updated.value,
            default_value=updated.default_value,
            bbox=BoundingBox(
                min_x=round(min_x, 2),
                min_y=round(min_y, 2),
                max_x=round(max_x, 2),
                max_y=round(max_y, 2),
                width=round(max_x - min_x, 2),
                height=round(max_y - min_y, 2),
            ),
            page_number=updated.page_number,
            options=updated.options,
            is_read_only=updated.is_read_only,
            is_required=updated.is_required,
            is_multiline=updated.is_multiline,
            max_length=updated.max_length,
        )
        return UpdateFormFieldResponse(
            status="ok",
            document_id=doc_id,
            field=field_model,
            message=f"Form field '{field_name}' successfully updated.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/pages/{page_idx}/rotate", response_model=RotatePageResponse)
@serialized_mutation
def rotate_page_endpoint(doc_id: str, page_idx: int, request: RotatePageRequest):
    """Rotates a specific page by the given degrees (0, 90, 180, 270, or relative offset)."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        new_rotation = doc.rotate_page(page_idx, request.degrees)
        return RotatePageResponse(
            success=True,
            document_id=doc_id,
            page_number=page_idx,
            new_rotation=new_rotation,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


def _split_part_count(doc, request: SplitDocumentRequest) -> int:
    """How many new sessions a split will create. Zero means the loop is a no-op.

    ``page_indices`` wins when both fields are set, matching the historical
    branch. A chunk split of a document with no pages creates nothing.
    """
    if request.page_indices is not None:
        return 1
    if request.chunk_size:
        total_pages = doc.page_count()
        if total_pages <= 0:
            return 0
        chunk_size = max(1, request.chunk_size)
        return (total_pages + chunk_size - 1) // chunk_size
    raise HTTPException(
        status_code=400,
        detail="Must specify either 'page_indices' or 'chunk_size'.",
    )


@app.post("/api/documents/{doc_id}/split", response_model=SplitDocumentResponse)
@serialized_mutation
def split_document_endpoint(doc_id: str, request: SplitDocumentRequest):
    """Extracts specified pages or splits the document into smaller chunks."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        part_count = _split_part_count(doc, request)
        extracted_ids = []
        if part_count == 0:
            return SplitDocumentResponse(
                success=True,
                source_document_id=doc_id,
                extracted_document_ids=extracted_ids,
                count=0,
            )

        # Each extracted document is charged the source size. Extraction copies
        # page objects, and the budget must fail closed before the loop starts.
        part_bytes = int(session.get("byte_size", 0))
        ensure_session_capacity(part_count, part_bytes * part_count)
        filename = session["filename"]
        if request.page_indices is not None:
            extracted_doc = doc.extract_pages(request.page_indices)
            new_id = bind_session(
                extracted_doc,
                f"{filename.replace('.pdf', '')}_extracted.pdf",
                part_bytes,
            )
            extracted_ids.append(new_id)
        else:
            total_pages = doc.page_count()
            chunk_size = max(1, request.chunk_size or 1)
            for start in range(0, total_pages, chunk_size):
                indices = list(range(start, min(start + chunk_size, total_pages)))
                chunk_doc = doc.extract_pages(indices)
                new_id = bind_session(
                    chunk_doc,
                    f"{filename.replace('.pdf', '')}_part_{len(extracted_ids)+1}.pdf",
                    part_bytes,
                )
                extracted_ids.append(new_id)

        return SplitDocumentResponse(
            success=True,
            source_document_id=doc_id,
            extracted_document_ids=extracted_ids,
            count=len(extracted_ids),
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/merge", response_model=MergeDocumentsResponse)
def merge_documents_endpoint(request: MergeDocumentsRequest):
    """Merges multiple existing document sessions in order into a new combined document."""
    if not request.document_ids:
        raise HTTPException(status_code=400, detail="At least one document ID must be provided.")

    try:
        with document_mutations(request.document_ids) as sessions:
            docs_to_merge = []
            accounted = 0
            for sess in sessions:
                docs_to_merge.append(sess["doc"])
                accounted += int(sess.get("byte_size", 0))
            ensure_session_capacity(1, accounted)
            merged_doc = pdf_engine.merge_documents(docs_to_merge)
            merged_id = bind_session(merged_doc, "merged_document.pdf", accounted)
            return MergeDocumentsResponse(
                success=True,
                merged_document_id=merged_id,
                filename="merged_document.pdf",
                page_count=merged_doc.page_count(),
            )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/pages/reorder", response_model=PageOperationResponse)
@serialized_mutation
def reorder_pages_endpoint(doc_id: str, request: ReorderPagesRequest):
    """Reorders the pages of a document according to a given permutation."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        order = request.new_order
        if order and min(order) == 1 and max(order) == len(order):
            order = [i - 1 for i in order]
        doc.reorder_pages(order)
        return PageOperationResponse(
            success=True,
            document_id=doc_id,
            page_count=doc.page_count(),
            message="Pages reordered successfully.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/pages/delete", response_model=PageOperationResponse)
@serialized_mutation
def delete_pages_endpoint(doc_id: str, request: DeletePagesRequest):
    """Deletes specified pages from a document."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        doc.delete_pages(request.page_indices)
        return PageOperationResponse(
            success=True,
            document_id=doc_id,
            page_count=doc.page_count(),
            message=f"Deleted {len(request.page_indices)} page(s) successfully.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


def annot_to_model(a) -> AnnotationModel:
    min_x, min_y, max_x, max_y = a.bbox()
    return AnnotationModel(
        id=a.id,
        page_index=a.page_index,
        page_number=a.page_number,
        subtype=a.subtype,
        bbox=BoundingBox(
            min_x=round(min_x, 2),
            min_y=round(min_y, 2),
            max_x=round(max_x, 2),
            max_y=round(max_y, 2),
            width=round(max_x - min_x, 2),
            height=round(max_y - min_y, 2),
        ),
        color=a.color,
        opacity=a.opacity,
        contents=a.contents,
        link_type=a.link_type,
        link_uri=a.link_uri,
        link_target_page=a.link_target_page,
        stamp_type=a.stamp_type,
        date_str=a.date_str,
    )


@app.get(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations",
    response_model=PageAnnotationsResponse,
)
def get_page_annotations_endpoint(doc_id: str, page_idx: int):
    """Retrieves all non-widget annotations (highlights, underlines, links, stamps) on a page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        annots = doc.get_page_annotations(page_idx)
        return PageAnnotationsResponse(
            document_id=doc_id,
            page_number=page_idx,
            count=len(annots),
            annotations=[annot_to_model(a) for a in annots],
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/markup",
    response_model=AnnotationActionResponse,
)
@serialized_mutation
def add_text_markup_endpoint(doc_id: str, page_idx: int, request: AddMarkupRequest):
    """Adds a text markup annotation (Highlight, Underline, StrikeOut) to a page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        annot_id = doc.add_text_markup(
            page_idx,
            request.subtype,
            request.min_x,
            request.min_y,
            request.max_x,
            request.max_y,
            request.color,
            request.opacity,
            request.contents,
        )
        return AnnotationActionResponse(
            success=True,
            document_id=doc_id,
            page_number=page_idx,
            annotation_id=annot_id,
            message=f"{request.subtype} markup annotation created successfully.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/link",
    response_model=AnnotationActionResponse,
)
@serialized_mutation
def add_link_endpoint(doc_id: str, page_idx: int, request: AddLinkRequest):
    """Adds an interactive URI link or internal GoTo link annotation to a page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        if request.uri:
            annot_id = doc.add_link_uri(
                page_idx,
                request.min_x,
                request.min_y,
                request.max_x,
                request.max_y,
                request.uri,
                request.show_border,
            )
            msg = f"Web link to '{request.uri}' created successfully."
        elif request.target_page is not None:
            annot_id = doc.add_link_goto(
                page_idx,
                request.min_x,
                request.min_y,
                request.max_x,
                request.max_y,
                request.target_page,
            )
            msg = f"Internal jump link to page {request.target_page} created successfully."
        else:
            raise HTTPException(status_code=400, detail="Either 'uri' or 'target_page' must be provided.")

        return AnnotationActionResponse(
            success=True,
            document_id=doc_id,
            page_number=page_idx,
            annotation_id=annot_id,
            message=msg,
        )
    except HTTPException:
        raise
    except Exception as e:
        refusal = _link_scheme_refusal(e)
        if refusal is not None:
            raise refusal
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/stamp",
    response_model=AnnotationActionResponse,
)
@serialized_mutation
def add_stamp_endpoint(doc_id: str, page_idx: int, request: AddStampRequest):
    """Adds a rubber stamp annotation with vector styling and text to a page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        annot_id = doc.add_stamp(
            page_idx,
            request.stamp_type,
            request.min_x,
            request.min_y,
            request.max_x,
            request.max_y,
            request.custom_text,
            request.color,
            request.date_str,
        )
        return AnnotationActionResponse(
            success=True,
            document_id=doc_id,
            page_number=page_idx,
            annotation_id=annot_id,
            message=f"Stamp '{request.stamp_type}' created successfully.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.delete(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/{annot_id}",
    response_model=AnnotationActionResponse,
)
@serialized_mutation
def delete_annotation_endpoint(doc_id: str, page_idx: int, annot_id: int):
    """Deletes an annotation from a page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        deleted = doc.delete_annotation(page_idx, annot_id)
        if not deleted:
            raise HTTPException(status_code=404, detail=f"Annotation {annot_id} not found on page {page_idx}.")
        return AnnotationActionResponse(
            success=True,
            document_id=doc_id,
            page_number=page_idx,
            annotation_id=annot_id,
            message=f"Annotation {annot_id} deleted successfully.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/annotations/flatten",
    response_model=FlattenAnnotationsResponse,
)
@serialized_mutation
def flatten_annotations_endpoint(doc_id: str, page_number: Optional[int] = None):
    """Permanently flattens visual annotations (highlights, underlines, stamps) into page content."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        flattened_count = doc.flatten_annotations(page_number)
        return FlattenAnnotationsResponse(
            success=True,
            document_id=doc_id,
            flattened_count=flattened_count,
            message=f"Flattened {flattened_count} annotation(s) into page graphics.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/pagination", response_model=WatermarkActionResponse)
@serialized_mutation
def add_pagination_endpoint(doc_id: str, request: AddPaginationRequest):
    """Applies dynamic Bates numbering or custom header/footer pagination across document pages."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        color_tuple = None
        if request.color and len(request.color) == 3:
            color_tuple = (float(request.color[0]), float(request.color[1]), float(request.color[2]))

        affected = doc.add_pagination(
            format=request.format,
            position=request.position,
            font_size=request.font_size,
            color=color_tuple,
            margin=request.margin,
            start_page_num=request.start_page_num,
            skip_first_page=request.skip_first_page,
            page_indices=request.page_indices,
        )
        return WatermarkActionResponse(
            success=True,
            document_id=doc_id,
            affected_pages=affected,
            message=f"Applied pagination across {affected} page(s).",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/watermark/text", response_model=WatermarkActionResponse)
@serialized_mutation
def add_text_watermark_endpoint(doc_id: str, request: AddTextWatermarkRequest):
    """Applies a semi-transparent rotated text watermark across document pages."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        color_tuple = None
        if request.color and len(request.color) == 3:
            color_tuple = (float(request.color[0]), float(request.color[1]), float(request.color[2]))

        affected = doc.add_text_watermark(
            text=request.text,
            font_size=request.font_size,
            color=color_tuple,
            opacity=request.opacity,
            rotation_degrees=request.rotation_degrees,
            placement=request.placement,
            page_indices=request.page_indices,
        )
        return WatermarkActionResponse(
            success=True,
            document_id=doc_id,
            affected_pages=affected,
            message=f"Applied text watermark across {affected} page(s).",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/watermark/image", response_model=WatermarkActionResponse)
@serialized_mutation
async def add_image_watermark_endpoint(
    doc_id: str,
    file: UploadFile = File(...),
    width: Optional[float] = Form(None),
    height: Optional[float] = Form(None),
    opacity: Optional[float] = Form(0.25),
    rotation_degrees: Optional[float] = Form(0.0),
    placement: Optional[str] = Form("background"),
    page_indices: Optional[str] = Form(None),
):
    """Embeds and applies a semi-transparent image watermark across document pages."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        image_bytes = await file.read()
        target_indices = None
        if page_indices:
            try:
                target_indices = [int(p.strip()) for p in page_indices.split(",") if p.strip()]
            except ValueError:
                raise HTTPException(status_code=400, detail="page_indices must be comma-separated integers.")

        affected = doc.add_image_watermark(
            image_bytes=image_bytes,
            width=width,
            height=height,
            opacity=opacity,
            rotation_degrees=rotation_degrees,
            placement=placement,
            page_indices=target_indices,
        )
        return WatermarkActionResponse(
            success=True,
            document_id=doc_id,
            affected_pages=affected,
            message=f"Applied image watermark across {affected} page(s).",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)



@app.websocket("/ws/documents/{doc_id}/pages/{page_idx}/reflow")
async def websocket_reflow(websocket: WebSocket, doc_id: str, page_idx: int, ticket: str = ""):
    """Interactive WebSocket endpoint streaming real-time typographic reflow as users type.

    The browser WebSocket constructor cannot set Authorization, so the caller
    presents a single-use ticket minted by POST /api/auth/ws-ticket.
    """
    subject = consume_ws_ticket(ticket)
    if subject is None:
        await websocket.accept()
        await websocket.close(code=1008, reason="Authentication required")
        return

    context_token = adopt_subject(subject)
    try:
        try:
            load_session(doc_id)
        except HTTPException:
            await websocket.accept()
            await websocket.close(code=1008, reason="Document session not found")
            return

        await websocket.accept()

        try:
            while True:
                data = await websocket.receive_json()
                paragraph_id = data.get("paragraph_id")
                new_text = data.get("text", "")

                if paragraph_id is None:
                    await websocket.send_json({"error": "Missing paragraph_id"})
                    continue

                try:
                    lock = await asyncio.to_thread(begin_document_mutation, doc_id)
                    try:
                        doc = load_session(doc_id)["doc"]
                        page = doc.get_page(page_idx)
                        page.edit_paragraph(int(paragraph_id), new_text)
                        doc.update_page(page)

                        paragraphs_raw = page.get_paragraphs()
                        updated_para = next((p for p in paragraphs_raw if p.id == paragraph_id), None)
                        if updated_para:
                            min_x, min_y, max_x, max_y = updated_para.bbox()
                            payload = {
                                "status": "ok",
                                "paragraph_id": paragraph_id,
                                "line_count": updated_para.line_count,
                                "text": updated_para.text,
                                "bbox": {
                                    "min_x": round(min_x, 2),
                                    "min_y": round(min_y, 2),
                                    "max_x": round(max_x, 2),
                                    "max_y": round(max_y, 2),
                                    "width": round(max_x - min_x, 2),
                                    "height": round(max_y - min_y, 2),
                                },
                            }
                        else:
                            payload = {"status": "ok", "paragraph_id": paragraph_id}
                    finally:
                        lock.release()
                    await websocket.send_json(payload)
                except HTTPException:
                    raise
                except Exception as e:
                    failure = _public_error(400, e)
                    await websocket.send_json({"status": "error", "message": failure.detail})
        except WebSocketDisconnect:
            pass
    finally:
        reset_subject(context_token)


@app.post(
    "/api/documents/{doc_id}/redact/regions",
    response_model=RedactionActionResponse,
)
@serialized_mutation
def redact_regions_endpoint(doc_id: str, request: RedactRegionsRequest):
    """Irreversibly excises text glyphs and draws opaque blackout boxes on target coordinates."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        regions_tuples = [(r.min_x, r.min_y, r.max_x, r.max_y) for r in request.regions]
        fill_tuple = tuple(request.fill_color) if request.fill_color and len(request.fill_color) == 3 else (0.0, 0.0, 0.0)
        text_tuple = tuple(request.text_color) if request.text_color and len(request.text_color) == 3 else (1.0, 1.0, 1.0)

        summary = doc.redact_regions(
            request.page_number,
            regions_tuples,
            fill_color=fill_tuple,
            overlay_text=request.overlay_text,
            text_color=text_tuple,
            font_size=request.font_size,
            prune_annotations=request.prune_annotations,
        )

        summary_model = RedactionSummaryModel(
            page_number=summary.page_number,
            purged_glyphs_count=summary.purged_glyphs_count,
            modified_blocks_count=summary.modified_blocks_count,
            blackout_boxes_count=summary.blackout_boxes_count,
            pruned_annotations_count=summary.pruned_annotations_count,
            applied_rects=[list(r) for r in summary.applied_rects],
        )
        record_document_action("redact", doc_id)

        return RedactionActionResponse(
            success=True,
            document_id=doc_id,
            total_purged_glyphs=summary.purged_glyphs_count,
            total_blackout_boxes=summary.blackout_boxes_count,
            total_pruned_annotations=summary.pruned_annotations_count,
            summaries=[summary_model],
            message=f"Applied {summary.blackout_boxes_count} redaction(s) on page {request.page_number} (purged {summary.purged_glyphs_count} glyphs).",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/redact/pattern",
    response_model=RedactionActionResponse,
)
@serialized_mutation
def redact_pattern_endpoint(doc_id: str, request: RedactPatternRequest):
    """Scans pages for sensitive PII (Email, Phone, SSN, Credit Card, RFC, CURP) and redacts matches."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        fill_tuple = tuple(request.fill_color) if request.fill_color and len(request.fill_color) == 3 else (0.0, 0.0, 0.0)
        text_tuple = tuple(request.text_color) if request.text_color and len(request.text_color) == 3 else (1.0, 1.0, 1.0)

        summaries = doc.redact_pattern(
            pattern_type=request.pattern_type,
            custom_query=request.custom_query,
            case_sensitive=request.case_sensitive,
            page_indices=request.page_numbers,
            fill_color=fill_tuple,
            overlay_text=request.overlay_text,
            text_color=text_tuple,
            font_size=request.font_size,
            prune_annotations=request.prune_annotations,
            scrub_metadata=request.scrub_metadata,
        )

        summary_models = [
            RedactionSummaryModel(
                page_number=s.page_number,
                purged_glyphs_count=s.purged_glyphs_count,
                modified_blocks_count=s.modified_blocks_count,
                blackout_boxes_count=s.blackout_boxes_count,
                pruned_annotations_count=s.pruned_annotations_count,
                applied_rects=[list(r) for r in s.applied_rects],
            )
            for s in summaries
        ]

        tot_glyphs = sum(s.purged_glyphs_count for s in summaries)
        tot_boxes = sum(s.blackout_boxes_count for s in summaries)
        tot_annots = sum(s.pruned_annotations_count for s in summaries)
        record_document_action("redact", doc_id)

        return RedactionActionResponse(
            success=True,
            document_id=doc_id,
            total_purged_glyphs=tot_glyphs,
            total_blackout_boxes=tot_boxes,
            total_pruned_annotations=tot_annots,
            summaries=summary_models,
            message=f"Redacted {tot_boxes} occurrence(s) across {len(summaries)} page(s) (purged {tot_glyphs} glyphs).",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/redact/text",
    response_model=RedactionActionResponse,
)
@serialized_mutation
def redact_text_endpoint(doc_id: str, request: RedactTextRequest):
    """Finds exact string matches across pages, removes them from content streams, and blacks them out."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        fill_tuple = tuple(request.fill_color) if request.fill_color and len(request.fill_color) == 3 else (0.0, 0.0, 0.0)
        text_tuple = tuple(request.text_color) if request.text_color and len(request.text_color) == 3 else (1.0, 1.0, 1.0)

        summaries = doc.redact_text(
            query=request.query,
            case_sensitive=request.case_sensitive,
            page_indices=request.page_numbers,
            fill_color=fill_tuple,
            overlay_text=request.overlay_text,
            text_color=text_tuple,
            font_size=request.font_size,
            prune_annotations=request.prune_annotations,
        )

        summary_models = [
            RedactionSummaryModel(
                page_number=s.page_number,
                purged_glyphs_count=s.purged_glyphs_count,
                modified_blocks_count=s.modified_blocks_count,
                blackout_boxes_count=s.blackout_boxes_count,
                pruned_annotations_count=s.pruned_annotations_count,
                applied_rects=[list(r) for r in s.applied_rects],
            )
            for s in summaries
        ]

        tot_glyphs = sum(s.purged_glyphs_count for s in summaries)
        tot_boxes = sum(s.blackout_boxes_count for s in summaries)
        tot_annots = sum(s.pruned_annotations_count for s in summaries)
        record_document_action("redact", doc_id)

        return RedactionActionResponse(
            success=True,
            document_id=doc_id,
            total_purged_glyphs=tot_glyphs,
            total_blackout_boxes=tot_boxes,
            total_pruned_annotations=tot_annots,
            summaries=summary_models,
            message=f"Redacted '{request.query}': {tot_boxes} occurrence(s) across {len(summaries)} page(s).",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/sanitize",
    response_model=SanitizeDocumentResponse,
)
@serialized_mutation
def sanitize_document_endpoint(doc_id: str, request: SanitizeDocumentRequest):
    """Purges sensitive document metadata (/Info dictionary and /Metadata XMP stream)."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        modified = doc.sanitize_document() if request.scrub_metadata else False
        return SanitizeDocumentResponse(
            success=True,
            document_id=doc_id,
            modified=modified,
            message="Document metadata sanitized successfully." if modified else "No metadata modified.",
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.get(
    "/api/documents/{doc_id}/security",
    response_model=SecurityStatusResponse,
)
def get_security_status_endpoint(doc_id: str):
    """Retrieves encryption state and each signature integrity check."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        is_enc = doc.is_encrypted()
        raw_sigs = doc.verify_signatures()
        sig_models = [
            SignatureModel(
                field_name=s.field_name,
                signer_name=s.signer_name,
                reason=s.reason,
                location=s.location,
                date=s.date,
                sub_filter=s.sub_filter,
                byte_range=s.byte_range,
                contents_hex=s.contents_hex,
                byte_range_valid=s.byte_range_valid,
                rect=list(s.rect),
                page_number=s.page_number,
            )
            for s in raw_sigs
        ]
        return SecurityStatusResponse(
            document_id=doc_id,
            is_encrypted=is_enc,
            signatures=sig_models,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/security/encrypt",
    response_model=SecurityActionResponse,
)
@serialized_mutation
def encrypt_document_endpoint(doc_id: str, request: EncryptDocumentRequest):
    """Encrypts document using AES-128 and password protection."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        perms = None
        if request.permissions is not None:
            p = request.permissions
            perms = pdf_engine.PdfPermissions(
                p.print_low_res,
                p.print_high_res,
                p.modify_contents,
                p.copy_extract,
                p.modify_annotations,
                p.fill_forms,
                p.accessibility_extract,
                p.assemble_document,
            )

        doc.encrypt(
            user_password=request.user_password,
            owner_password=request.owner_password,
            permissions=perms,
            encrypt_metadata=request.encrypt_metadata if request.encrypt_metadata is not None else True,
        )
        return SecurityActionResponse(
            success=True,
            document_id=doc_id,
            message="Document successfully encrypted with AES-128.",
            is_encrypted=True,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/security/decrypt",
    response_model=SecurityActionResponse,
)
@serialized_mutation
def decrypt_document_endpoint(doc_id: str, request: DecryptDocumentRequest):
    """Decrypts document using the provided password."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        doc.decrypt(request.password)
        return SecurityActionResponse(
            success=True,
            document_id=doc_id,
            message="Document successfully decrypted.",
            is_encrypted=False,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post(
    "/api/documents/{doc_id}/security/sign",
    response_model=SecurityActionResponse,
)
@serialized_mutation
def sign_document_endpoint(doc_id: str, request: SignDocumentRequest):
    """Stamps a SHA-256 attestation, or a detached PKCS#7 signature when a certificate is supplied.

    A timestamp is requested only after the signature exists. A failed timestamp
    restores the file from before the signature. The audit row is written after
    the whole operation succeeds.
    """
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        pkcs12_der = _prepare_signing_material(request)
        cms = _uses_signing_material(request)
        rect = request.rect if request.rect and len(request.rect) == 4 else [72.0, 72.0, 272.0, 142.0]
        snapshot = bytes(doc.save_to_bytes()) if cms else None
        try:
            kwargs = {}
            if request.certificate_pem is not None:
                kwargs["certificate_pem"] = request.certificate_pem
            if request.private_key_pem is not None:
                kwargs["private_key_pem"] = request.private_key_pem
            if request.chain_pem is not None:
                kwargs["chain_pem"] = request.chain_pem
            if pkcs12_der is not None:
                kwargs["pkcs12_der"] = pkcs12_der
                kwargs["pkcs12_password"] = request.pkcs12_password.encode("utf-8")
            if request.tsa_url:
                kwargs["reserve_timestamp"] = True
            sig = doc.sign(
                request.signer_name,
                request.reason,
                request.location,
                rect,
                request.page_number,
                request.contact_info,
                **kwargs,
            )
            if request.tsa_url:
                query = None
                try:
                    query = bytes(doc.cms_timestamp_request())
                    token = tsa.fetch_timestamp(request.tsa_url, query)
                    sig = doc.embed_cms_timestamp(token)
                except HTTPException:
                    _rollback_document(doc, snapshot)
                    raise
                except ValueError as exc:
                    _rollback_document(doc, snapshot)
                    detail = str(exc)
                    if detail not in (
                        "Timestamp authority URL was rejected.",
                        "Timestamp authority request failed.",
                    ):
                        detail = "Timestamp authority request failed."
                    raise HTTPException(status_code=400, detail=detail)
                except Exception as exc:
                    _rollback_document(doc, snapshot)
                    if query is None:
                        raise _signing_failure(exc)
                    raise HTTPException(
                        status_code=400,
                        detail="Timestamp token was rejected.",
                    )
        except HTTPException:
            raise
        except Exception as exc:
            _rollback_document(doc, snapshot)
            raise _signing_failure(exc) from None
        record_document_action("sign", doc_id)
        if request.tsa_url:
            message = (
                f"PKCS#7 detached signature and RFC 3161 timestamp created for {request.signer_name}."
            )
        elif cms:
            message = f"PKCS#7 detached signature created for {request.signer_name}."
        else:
            message = f"SHA-256 byte-range attestation created for {request.signer_name}."
        return SecurityActionResponse(
            success=True,
            document_id=doc_id,
            message=message,
            signature=SignatureModel(
                field_name=sig.field_name,
                signer_name=sig.signer_name,
                reason=sig.reason,
                location=sig.location,
                date=sig.date,
                sub_filter=sig.sub_filter,
                byte_range=sig.byte_range,
                contents_hex=sig.contents_hex,
                byte_range_valid=sig.byte_range_valid,
                rect=list(sig.rect),
                page_number=sig.page_number,
            ),
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.get(
    "/api/documents/{doc_id}/security/signatures",
    response_model=List[SignatureModel],
)
def get_signatures_endpoint(doc_id: str):
    """Lists each signature dictionary and whether its integrity check still matches."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        raw_sigs = doc.verify_signatures()
        return [
            SignatureModel(
                field_name=s.field_name,
                signer_name=s.signer_name,
                reason=s.reason,
                location=s.location,
                date=s.date,
                sub_filter=s.sub_filter,
                byte_range=s.byte_range,
                contents_hex=s.contents_hex,
                byte_range_valid=s.byte_range_valid,
                rect=list(s.rect),
                page_number=s.page_number,
            )
            for s in raw_sigs
        ]
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.get(
    "/api/documents/{doc_id}/pages/{page_idx}/tables",
    response_model=PageTablesResponse,
)
def get_page_tables_endpoint(doc_id: str, page_idx: int):
    """Detects and extracts all structured tables on the specified page."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        raw_tables = doc.extract_tables(page_idx)
        tables = []
        for t in raw_tables:
            cells = [
                TableCellModel(
                    row=c.row,
                    col=c.col,
                    row_span=c.row_span,
                    col_span=c.col_span,
                    text=c.text,
                    is_header=c.is_header,
                    bbox=BoundingBox(
                        min_x=c.min_x,
                        min_y=c.min_y,
                        max_x=c.max_x,
                        max_y=c.max_y,
                        width=c.max_x - c.min_x,
                        height=c.max_y - c.min_y,
                    ),
                )
                for c in t.cells
            ]
            tables.append(
                TableModel(
                    table_idx=t.table_idx,
                    page_number=t.page_number,
                    row_count=t.row_count,
                    col_count=t.col_count,
                    bbox=BoundingBox(
                        min_x=t.min_x,
                        min_y=t.min_y,
                        max_x=t.max_x,
                        max_y=t.max_y,
                        width=t.max_x - t.min_x,
                        height=t.max_y - t.min_y,
                    ),
                    headers=list(t.headers),
                    rows=[list(r) for r in t.rows],
                    cells=cells,
                )
            )

        return PageTablesResponse(
            document_id=doc_id,
            page_number=page_idx,
            total_tables=len(tables),
            tables=tables,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.get(
    "/api/documents/{doc_id}/pages/{page_idx}/tables/{table_idx}/export",
    response_model=TableExportResponse,
)
def export_table_endpoint(
    doc_id: str,
    page_idx: int,
    table_idx: int,
    format: str = "csv",
    download: bool = False,
):
    """Exports a detected table as csv, json, markdown, or escaped html text.

    An html download is an attachment of plain text. The studio does not
    parse that body as a document.
    """
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        content = doc.export_table(page_idx, table_idx, format)
        raw_tables = doc.extract_tables(page_idx)
        t = raw_tables[table_idx] if table_idx < len(raw_tables) else None
        row_count = t.row_count if t else 0
        col_count = t.col_count if t else 0

        if download:
            media_types = {
                "csv": "text/csv; charset=utf-8",
                "json": "application/json",
                "markdown": "text/markdown; charset=utf-8",
                "md": "text/markdown; charset=utf-8",
                "html": "text/plain; charset=utf-8",
            }
            media_type = media_types.get(format.lower(), "text/plain; charset=utf-8")
            extensions = {"csv": "csv", "json": "json", "markdown": "md", "md": "md", "html": "html"}
            ext = extensions.get(format.lower(), "txt")
            filename = f"table_p{page_idx}_{table_idx}.{ext}"
            return Response(
                content=content,
                media_type=media_type,
                headers={
                    "Content-Disposition": _content_disposition("attachment", filename),
                    "X-Content-Type-Options": "nosniff",
                },
            )

        return TableExportResponse(
            document_id=doc_id,
            page_number=page_idx,
            table_idx=table_idx,
            format=format,
            content=content,
            row_count=row_count,
            col_count=col_count,
        )
    except HTTPException:
        raise
    except Exception as e:
        raise _public_error(400, e)


@app.post("/api/documents/{doc_id}/optimize", response_model=OptimizeResponse)
@serialized_mutation
async def optimize_document_endpoint(
    doc_id: str,
    request: OptimizeRequest,
):
    """Optimizes the PDF document using garbage collection, lossless Flate stream recompression,
    stream deduplication, and Object Stream (/ObjStm) packing."""
    session = load_session(doc_id)

    doc = session["doc"]
    try:
        opt_bytes, stats = doc.save_optimized_to_bytes(
            remove_unused=request.remove_unused,
            pack_object_streams=request.pack_object_streams,
            recompress_flate=request.recompress_flate,
            deduplicate_streams=request.deduplicate_streams,
            max_objects_per_stream=request.max_objects_per_stream,
        )

        # Charge the optimized file in place of the upload. The original upload
        # buffer is not retained. The export path still returns optimized_bytes.
        set_session_byte_size(doc_id, len(opt_bytes))
        session.pop("raw_bytes", None)
        session["doc"] = pdf_engine.Document.from_bytes(opt_bytes)
        session["optimized_bytes"] = opt_bytes
        session["original_size"] = stats.original_size
        session["optimized_size"] = stats.optimized_size
        record_document_action("optimize", doc_id)

        return OptimizeResponse(
            success=True,
            document_id=doc_id,
            original_size=stats.original_size,
            optimized_size=stats.optimized_size,
            bytes_saved=stats.bytes_saved,
            compression_ratio_pct=stats.compression_ratio_pct,
            objects_removed=stats.objects_removed,
            streams_recompressed=stats.streams_recompressed,
            object_streams_created=stats.object_streams_created,
            streams_deduplicated=stats.streams_deduplicated,
            message=(
                f"Optimización completada con éxito. Reducción del {stats.compression_ratio_pct:.1f}% "
                f"({stats.bytes_saved} bytes ahorrados, {stats.objects_removed} objetos purgados, "
                f"{stats.object_streams_created} flujos /ObjStm creados)."
            ),
        )
    except HTTPException:
        raise
    except Exception as e:
        refusal = _optimization_refusal(e)
        if refusal is not None:
            raise refusal
        raise _public_error(400, e)




