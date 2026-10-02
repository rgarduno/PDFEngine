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


class FormFieldModel(BaseModel):
    """Interactive AcroForm field definition."""
    id: int
    name: str
    alt_name: Optional[str] = None
    field_type: str
    value: str
    default_value: Optional[str] = None
    bbox: BoundingBox
    page_number: int
    options: List[str] = Field(default_factory=list)
    is_read_only: bool = False
    is_required: bool = False
    is_multiline: bool = False
    max_length: Optional[int] = None


class DocumentFormsResponse(BaseModel):
    """Response containing all extracted interactive form fields in the document."""
    document_id: str
    count: int
    fields: List[FormFieldModel]


class FillFormFieldRequest(BaseModel):
    """Request payload to fill a single form field."""
    name_or_id: str
    value: str


class BatchFillFormsRequest(BaseModel):
    """Request payload to fill multiple form fields by name in batch."""
    fields: dict[str, str]


class FillFormsResponse(BaseModel):
    """Response returned upon filling form fields."""
    success: bool
    document_id: str
    updated_count: int
    message: str


class FlattenFormsResponse(BaseModel):
    """Response returned upon permanently flattening form fields into page vectors."""
    success: bool
    document_id: str
    flattened_count: int
    message: str


class RotatePageRequest(BaseModel):
    """Request payload to rotate a page."""
    degrees: int = Field(default=90, description="Degrees to rotate: 90, 180, 270, or relative offset")


class RotatePageResponse(BaseModel):
    """Response returned after page rotation."""
    success: bool
    document_id: str
    page_number: int
    new_rotation: int


class SplitDocumentRequest(BaseModel):
    """Request payload to extract or split pages from a document."""
    page_indices: Optional[List[int]] = None
    chunk_size: Optional[int] = None


class SplitDocumentResponse(BaseModel):
    """Response returned after splitting document."""
    success: bool
    source_document_id: str
    extracted_document_ids: List[str]
    count: int


class MergeDocumentsRequest(BaseModel):
    """Request payload to merge multiple documents into one."""
    document_ids: List[str] = Field(description="Ordered list of document IDs to merge")


class MergeDocumentsResponse(BaseModel):
    """Response returned after merging documents."""
    success: bool
    merged_document_id: str
    filename: str
    page_count: int


class ReorderPagesRequest(BaseModel):
    """Request payload to reorder pages in a document."""
    new_order: List[int] = Field(description="Permutation of 0-based or 1-based page indices")


class DeletePagesRequest(BaseModel):
    """Request payload to delete pages from a document."""
    page_indices: List[int] = Field(description="List of 0-based or 1-based page indices to delete")


class PageOperationResponse(BaseModel):
    """General response for page assembly/manipulation operations."""
    success: bool
    document_id: str
    page_count: int
    message: str


class AnnotationModel(BaseModel):
    """Structured representation of a PDF annotation."""
    id: int
    page_index: int
    page_number: int
    subtype: str
    bbox: BoundingBox
    color: Optional[List[float]] = None
    opacity: float = 1.0
    contents: Optional[str] = None
    link_type: Optional[str] = None
    link_uri: Optional[str] = None
    link_target_page: Optional[int] = None
    stamp_type: Optional[str] = None
    date_str: Optional[str] = None


class PageAnnotationsResponse(BaseModel):
    """Response containing annotations on a given page."""
    document_id: str
    page_number: int
    count: int
    annotations: List[AnnotationModel]


class AddMarkupRequest(BaseModel):
    """Request payload to add a text markup annotation (Highlight, Underline, StrikeOut)."""
    subtype: str = Field(default="Highlight", description="Highlight, Underline, or StrikeOut")
    min_x: float
    min_y: float
    max_x: float
    max_y: float
    color: Optional[List[float]] = None
    opacity: Optional[float] = None
    contents: Optional[str] = None


class AddLinkRequest(BaseModel):
    """Request payload to add an interactive clickable link or GoTo destination."""
    min_x: float
    min_y: float
    max_x: float
    max_y: float
    uri: Optional[str] = None
    target_page: Optional[int] = None
    show_border: bool = False


class AddStampRequest(BaseModel):
    """Request payload to add a rubber stamp annotation."""
    stamp_type: str = Field(default="Approved", description="Approved, Confidential, Draft, Rejected, Final, TopSecret, or Custom")
    min_x: Optional[float] = None
    min_y: Optional[float] = None
    max_x: Optional[float] = None
    max_y: Optional[float] = None
    custom_text: Optional[str] = None
    color: Optional[List[float]] = None
    date_str: Optional[str] = None


class AnnotationActionResponse(BaseModel):
    """Response returned upon creating or modifying an annotation."""
    success: bool
    document_id: str
    page_number: int
    annotation_id: int
    message: str


class FlattenAnnotationsResponse(BaseModel):
    """Response returned upon flattening visual annotations into page vectors."""
    success: bool
    document_id: str
    flattened_count: int
    message: str


class AddPaginationRequest(BaseModel):
    """Request payload to apply dynamic pagination, headers, or footers."""
    format: Optional[str] = Field(default="Página {page} de {total}", description="Format template with {page} and {total}")
    position: Optional[str] = Field(default="bottom_center", description="top_left, top_center, top_right, bottom_left, bottom_center, bottom_right")
    font_size: Optional[float] = Field(default=9.0, description="Font size in points")
    color: Optional[List[float]] = Field(default=[0.35, 0.35, 0.35], description="RGB color array [r, g, b] in [0.0, 1.0]")
    margin: Optional[float] = Field(default=36.0, description="Margin distance from page edge")
    start_page_num: Optional[int] = Field(default=1, description="Starting page count number")
    skip_first_page: Optional[bool] = Field(default=False, description="Whether to omit numbering on first page")
    page_indices: Optional[List[int]] = Field(default=None, description="Specific 0-based page indices or null for all")


class AddTextWatermarkRequest(BaseModel):
    """Request payload to apply a semi-transparent text watermark."""
    text: str = Field(default="CONFIDENCIAL", description="Watermark text")
    font_size: Optional[float] = Field(default=52.0, description="Font size in points")
    color: Optional[List[float]] = Field(default=[0.80, 0.20, 0.20], description="RGB color array [r, g, b]")
    opacity: Optional[float] = Field(default=0.22, description="Alpha opacity between 0.0 and 1.0")
    rotation_degrees: Optional[float] = Field(default=45.0, description="Rotation angle in degrees")
    placement: Optional[str] = Field(default="background", description="background or foreground")
    page_indices: Optional[List[int]] = Field(default=None, description="Specific 0-based page indices or null for all")


class WatermarkActionResponse(BaseModel):
    """Response returned upon applying pagination or watermarks."""
    success: bool
    document_id: str
    affected_pages: int
    message: str

