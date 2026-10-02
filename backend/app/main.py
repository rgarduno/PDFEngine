"""PDFEngine Commercial REST API.

High-performance FastAPI service providing document ingestion,
interactive scene graph layout inspection, and surgical in-place PDF editing.
"""

import uuid
from typing import Dict
from fastapi import FastAPI, File, HTTPException, UploadFile, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import Response

try:
    import pdf_engine
except ImportError:
    pdf_engine = None

from app.models import (
    BoundingBox,
    DocumentUploadResponse,
    EditParagraphRequest,
    EditParagraphResponse,
    ImageModel,
    PageImagesResponse,
    PageSceneGraph,
    ParagraphModel,
)

app = FastAPI(
    title="PDFEngine API",
    description="High-performance, lossless PDF parsing and surgical editing service",
    version="0.1.0",
)

# Enable CORS for web and client frontends
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Active document sessions stored in memory (stateless worker pattern)
DOCUMENT_SESSIONS: Dict[str, dict] = {}


@app.get("/api/health")
def health_check():
    """Health check endpoint for container orchestrators and load balancers."""
    return {
        "status": "healthy",
        "engine_loaded": pdf_engine is not None,
        "version": "0.1.0",
    }


@app.post("/api/documents/upload", response_model=DocumentUploadResponse)
async def upload_document(file: UploadFile = File(...)):
    """Uploads a PDF document, indexes pages, and initializes an editing session."""
    if not file.filename.lower().endswith(".pdf"):
        raise HTTPException(status_code=400, detail="Only PDF documents are supported.")

    content = await file.read()
    if len(content) < 8 or not content.startswith(b"%PDF-"):
        raise HTTPException(status_code=400, detail="Uploaded file is not a valid PDF.")

    if pdf_engine is None:
        raise HTTPException(
            status_code=500, detail="Native pdf_engine core extension is not loaded."
        )

    try:
        doc = pdf_engine.Document.from_bytes(content)
    except Exception as e:
        raise HTTPException(status_code=422, detail=f"Failed to parse PDF document: {e}")

    doc_id = str(uuid.uuid4())
    page_count = doc.page_count()

    DOCUMENT_SESSIONS[doc_id] = {
        "doc": doc,
        "filename": file.filename,
        "raw_bytes": content,
    }

    return DocumentUploadResponse(
        document_id=doc_id,
        filename=file.filename,
        page_count=page_count,
    )


@app.get("/api/documents/{doc_id}/pages/{page_idx}/scenegraph", response_model=PageSceneGraph)
def get_page_scenegraph(doc_id: str, page_idx: int):
    """Retrieves the layout scene graph (paragraphs, bounding boxes, alignments) for a page."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        page = doc.get_page(page_idx)
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e))

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


@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/edit/{paragraph_id}",
    response_model=EditParagraphResponse,
)
def edit_paragraph(
    doc_id: str, page_idx: int, paragraph_id: int, request: EditParagraphRequest
):
    """Performs an in-place surgical paragraph edit, recalculating reflow without altering graphics."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        page = doc.get_page(page_idx)
        page.edit_paragraph(paragraph_id, request.new_text)
        doc.update_page(page)
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Surgical edit error: {e}")

    return EditParagraphResponse(
        success=True,
        document_id=doc_id,
        page_number=page_idx,
        paragraph_id=paragraph_id,
        updated_text=request.new_text,
        message="Paragraph surgically replaced with zero layout drift.",
    )


@app.get("/api/documents/{doc_id}/export")
def export_document(doc_id: str):
    """Serializes the edited document into a downloadable PDF binary."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        pdf_bytes = doc.save_to_bytes()
    except Exception as e:
        raise HTTPException(status_code=500, detail=f"Failed to serialize PDF: {e}")

    filename = session.get("filename", "document.pdf")
    base_name = filename.rsplit(".", 1)[0]
    export_name = f"{base_name}_edited.pdf"

    return Response(
        content=bytes(pdf_bytes),
        media_type="application/pdf",
        headers={"Content-Disposition": f'attachment; filename="{export_name}"'},
    )


@app.get("/api/documents/{doc_id}/pages/{page_idx}/fonts")
def list_page_fonts(doc_id: str, page_idx: int):
    """Lists all font resource identifiers declared on a specific page."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        fonts = doc.get_page_fonts(page_idx)
        return {
            "page_number": page_idx,
            "fonts": list(fonts.keys()),
            "embedded_count": len(fonts),
        }
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e))


@app.get("/api/documents/{doc_id}/pages/{page_idx}/fonts/{font_name}")
def get_page_font_binary(doc_id: str, page_idx: int, font_name: str):
    """Extracts and streams embedded TrueType/OpenType font binaries for dynamic browser @font-face registration."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        fonts = doc.get_page_fonts(page_idx)
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e))

    font_bytes = fonts.get(font_name)
    if not font_bytes:
        clean_name = font_name.lstrip("/")
        font_bytes = fonts.get(clean_name)

    if not font_bytes:
        raise HTTPException(
            status_code=404,
            detail=f"Font '{font_name}' has no embedded binary on page {page_idx}.",
        )

    return Response(
        content=bytes(font_bytes),
        media_type="font/ttf",
        headers={
            "Content-Disposition": f'inline; filename="{font_name}.ttf"',
            "Cache-Control": "public, max-age=86400",
        },
    )


@app.get(
    "/api/documents/{doc_id}/pages/{page_idx}/images",
    response_model=PageImagesResponse,
)
def get_page_images(doc_id: str, page_idx: int):
    """Lists all Image XObjects declared and placed on a specific page."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e))


@app.get("/api/documents/{doc_id}/images/{image_id}")
def get_image_binary(doc_id: str, image_id: int):
    """Extracts and streams raw image binary (JPEG or PNG) for browser display."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
    except Exception as e:
        raise HTTPException(status_code=404, detail=f"Image {image_id} error: {e}")


@app.post("/api/documents/{doc_id}/images/{image_id}/replace")
async def replace_image(doc_id: str, image_id: int, file: UploadFile = File(...)):
    """Surgically replaces an existing image XObject in the PDF with a new JPEG or PNG file."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to replace image: {e}")


@app.websocket("/ws/documents/{doc_id}/pages/{page_idx}/reflow")
async def websocket_reflow(websocket: WebSocket, doc_id: str, page_idx: int):
    """Interactive WebSocket endpoint streaming real-time typographic reflow as users type."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        await websocket.close(code=1008, reason="Document session not found")
        return

    await websocket.accept()
    doc = session["doc"]

    try:
        while True:
            data = await websocket.receive_json()
            paragraph_id = data.get("paragraph_id")
            new_text = data.get("text", "")

            if paragraph_id is None:
                await websocket.send_json({"error": "Missing paragraph_id"})
                continue

            try:
                page = doc.get_page(page_idx)
                page.edit_paragraph(int(paragraph_id), new_text)
                doc.update_page(page)

                paragraphs_raw = page.get_paragraphs()
                updated_para = next((p for p in paragraphs_raw if p.id == paragraph_id), None)
                if updated_para:
                    min_x, min_y, max_x, max_y = updated_para.bbox()
                    await websocket.send_json({
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
                    })
                else:
                    await websocket.send_json({"status": "ok", "paragraph_id": paragraph_id})
            except Exception as e:
                await websocket.send_json({"status": "error", "message": str(e)})
    except WebSocketDisconnect:
        pass

