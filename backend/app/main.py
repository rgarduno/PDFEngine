"""PDFEngine Commercial REST API.

High-performance FastAPI service providing document ingestion,
interactive scene graph layout inspection, and surgical in-place PDF editing.
"""

import uuid
from typing import Dict, Optional
from fastapi import FastAPI, File, HTTPException, UploadFile, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import Response

try:
    import pdf_engine
except ImportError:
    pdf_engine = None

from app.models import (
    AddLinkRequest,
    AddMarkupRequest,
    AddStampRequest,
    AnnotationActionResponse,
    AnnotationModel,
    BatchFillFormsRequest,
    BoundingBox,
    DeletePagesRequest,
    DocumentFormsResponse,
    DocumentUploadResponse,
    EditParagraphRequest,
    EditParagraphResponse,
    FillFormsResponse,
    FlattenAnnotationsResponse,
    FlattenFormsResponse,
    FormFieldModel,
    ImageModel,
    MergeDocumentsRequest,
    MergeDocumentsResponse,
    PageAnnotationsResponse,
    PageImagesResponse,
    PageOperationResponse,
    PageSceneGraph,
    ParagraphModel,
    ReorderPagesRequest,
    RotatePageRequest,
    RotatePageResponse,
    SplitDocumentRequest,
    SplitDocumentResponse,
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


@app.get("/api/documents/{doc_id}/forms", response_model=DocumentFormsResponse)
def get_document_forms(doc_id: str):
    """Retrieves all interactive AcroForm fields present in the document."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to extract form fields: {e}")


@app.post("/api/documents/{doc_id}/forms/fill", response_model=FillFormsResponse)
def fill_document_forms(doc_id: str, request: BatchFillFormsRequest):
    """Fills one or more interactive form fields by name in a batch transaction."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        updated_count = doc.fill_form_fields(request.fields)
        return FillFormsResponse(
            success=True,
            document_id=doc_id,
            updated_count=updated_count,
            message=f"Successfully filled {updated_count} form field(s).",
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to fill form fields: {e}")


@app.post("/api/documents/{doc_id}/forms/flatten", response_model=FlattenFormsResponse)
def flatten_document_forms_endpoint(doc_id: str):
    """Permanently flattens all interactive form fields into page vectors and strips widget annotations."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        flattened_count = doc.flatten_forms()
        return FlattenFormsResponse(
            success=True,
            document_id=doc_id,
            flattened_count=flattened_count,
            message=f"Successfully flattened {flattened_count} form field(s) into permanent page graphics.",
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to flatten form fields: {e}")


@app.post("/api/documents/{doc_id}/pages/{page_idx}/rotate", response_model=RotatePageResponse)
def rotate_page_endpoint(doc_id: str, page_idx: int, request: RotatePageRequest):
    """Rotates a specific page by the given degrees (0, 90, 180, 270, or relative offset)."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        new_rotation = doc.rotate_page(page_idx, request.degrees)
        return RotatePageResponse(
            success=True,
            document_id=doc_id,
            page_number=page_idx,
            new_rotation=new_rotation,
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to rotate page: {e}")


@app.post("/api/documents/{doc_id}/split", response_model=SplitDocumentResponse)
def split_document_endpoint(doc_id: str, request: SplitDocumentRequest):
    """Extracts specified pages or splits the document into smaller chunks."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        extracted_ids = []
        if request.page_indices is not None:
            extracted_doc = doc.extract_pages(request.page_indices)
            new_id = str(uuid.uuid4())
            DOCUMENT_SESSIONS[new_id] = {
                "doc": extracted_doc,
                "filename": f"{session['filename'].replace('.pdf', '')}_extracted.pdf",
            }
            extracted_ids.append(new_id)
        elif request.chunk_size:
            total_pages = doc.page_count()
            chunk_size = max(1, request.chunk_size)
            for start in range(0, total_pages, chunk_size):
                indices = list(range(start, min(start + chunk_size, total_pages)))
                chunk_doc = doc.extract_pages(indices)
                new_id = str(uuid.uuid4())
                DOCUMENT_SESSIONS[new_id] = {
                    "doc": chunk_doc,
                    "filename": f"{session['filename'].replace('.pdf', '')}_part_{len(extracted_ids)+1}.pdf",
                }
                extracted_ids.append(new_id)
        else:
            raise HTTPException(status_code=400, detail="Must specify either 'page_indices' or 'chunk_size'.")

        return SplitDocumentResponse(
            success=True,
            source_document_id=doc_id,
            extracted_document_ids=extracted_ids,
            count=len(extracted_ids),
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to split document: {e}")


@app.post("/api/documents/merge", response_model=MergeDocumentsResponse)
def merge_documents_endpoint(request: MergeDocumentsRequest):
    """Merges multiple existing document sessions in order into a new combined document."""
    if not request.document_ids:
        raise HTTPException(status_code=400, detail="At least one document ID must be provided.")

    docs_to_merge = []
    for d_id in request.document_ids:
        sess = DOCUMENT_SESSIONS.get(d_id)
        if not sess:
            raise HTTPException(status_code=404, detail=f"Document session '{d_id}' not found.")
        docs_to_merge.append(sess["doc"])

    try:
        merged_doc = pdf_engine.merge_documents(docs_to_merge)
        merged_id = str(uuid.uuid4())
        DOCUMENT_SESSIONS[merged_id] = {
            "doc": merged_doc,
            "filename": "merged_document.pdf",
        }
        return MergeDocumentsResponse(
            success=True,
            merged_document_id=merged_id,
            filename="merged_document.pdf",
            page_count=merged_doc.page_count(),
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to merge documents: {e}")


@app.post("/api/documents/{doc_id}/pages/reorder", response_model=PageOperationResponse)
def reorder_pages_endpoint(doc_id: str, request: ReorderPagesRequest):
    """Reorders the pages of a document according to a given permutation."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to reorder pages: {e}")


@app.post("/api/documents/{doc_id}/pages/delete", response_model=PageOperationResponse)
def delete_pages_endpoint(doc_id: str, request: DeletePagesRequest):
    """Deletes specified pages from a document."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        doc.delete_pages(request.page_indices)
        return PageOperationResponse(
            success=True,
            document_id=doc_id,
            page_count=doc.page_count(),
            message=f"Deleted {len(request.page_indices)} page(s) successfully.",
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to delete pages: {e}")


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
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        annots = doc.get_page_annotations(page_idx)
        return PageAnnotationsResponse(
            document_id=doc_id,
            page_number=page_idx,
            count=len(annots),
            annotations=[annot_to_model(a) for a in annots],
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to retrieve annotations: {e}")


@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/markup",
    response_model=AnnotationActionResponse,
)
def add_text_markup_endpoint(doc_id: str, page_idx: int, request: AddMarkupRequest):
    """Adds a text markup annotation (Highlight, Underline, StrikeOut) to a page."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to add markup annotation: {e}")


@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/link",
    response_model=AnnotationActionResponse,
)
def add_link_endpoint(doc_id: str, page_idx: int, request: AddLinkRequest):
    """Adds an interactive URI link or internal GoTo link annotation to a page."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
        raise HTTPException(status_code=400, detail=f"Failed to add link annotation: {e}")


@app.post(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/stamp",
    response_model=AnnotationActionResponse,
)
def add_stamp_endpoint(doc_id: str, page_idx: int, request: AddStampRequest):
    """Adds a rubber stamp annotation with vector styling and text to a page."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to add stamp annotation: {e}")


@app.delete(
    "/api/documents/{doc_id}/pages/{page_idx}/annotations/{annot_id}",
    response_model=AnnotationActionResponse,
)
def delete_annotation_endpoint(doc_id: str, page_idx: int, annot_id: int):
    """Deletes an annotation from a page."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

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
        raise HTTPException(status_code=400, detail=f"Failed to delete annotation: {e}")


@app.post(
    "/api/documents/{doc_id}/annotations/flatten",
    response_model=FlattenAnnotationsResponse,
)
def flatten_annotations_endpoint(doc_id: str, page_number: Optional[int] = None):
    """Permanently flattens visual annotations (highlights, underlines, stamps) into page content."""
    session = DOCUMENT_SESSIONS.get(doc_id)
    if not session:
        raise HTTPException(status_code=404, detail="Document session not found.")

    doc = session["doc"]
    try:
        flattened_count = doc.flatten_annotations(page_number)
        return FlattenAnnotationsResponse(
            success=True,
            document_id=doc_id,
            flattened_count=flattened_count,
            message=f"Flattened {flattened_count} annotation(s) into page graphics.",
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"Failed to flatten annotations: {e}")


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

