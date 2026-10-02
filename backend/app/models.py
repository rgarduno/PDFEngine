"""Pydantic data models for the PDFEngine REST API.

Defines serialized representations for document metadata, layout scene graphs,
and surgical editing requests.
"""

from typing import List, Optional
from pydantic import BaseModel, Field


class BoundingBox(BaseModel):
    """Bounding box coordinates in PDF page points."""
    min_x: float
    min_y: float
    max_x: float
    max_y: float
    width: float
    height: float


class ParagraphModel(BaseModel):
    """Extracted paragraph with typographic layout attributes."""
    id: int
    text: str
    bbox: BoundingBox
    alignment: str = Field(description="left, center, right, or justified")
    leading: float
    line_count: int


class PageSceneGraph(BaseModel):
    """Complete layout scene graph for a page."""
    page_number: int
    paragraphs: List[ParagraphModel]


class DocumentUploadResponse(BaseModel):
    """Response returned upon successful document upload."""
    document_id: str
    filename: str
    page_count: int


class EditParagraphRequest(BaseModel):
    """Payload to perform an in-place surgical paragraph edit."""
    new_text: str


class EditParagraphResponse(BaseModel):
    """Response returned after surgical edit execution."""
    success: bool
    document_id: str
    page_number: int
    paragraph_id: int
    updated_text: str
    message: str


class ImageModel(BaseModel):
    """Extracted image XObject on a page with spatial layout attributes."""
    id: int
    name: str
    width_px: int
    height_px: int
    color_space: str
    bits_per_component: int
    filter: Optional[str] = None
    byte_size: int
    bbox: BoundingBox


class PageImagesResponse(BaseModel):
    """Response containing all extracted images on a page."""
    page_number: int
    images: List[ImageModel]
    count: int

