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


class PageOverviewItem(BaseModel):
    """Overview metadata and layout preview for a single page in thumbnail navigation."""
    page_number: int
    page_index: int
    rotation: int
    paragraph_count: int
    preview_snippet: str
    width: float = 612.0
    height: float = 792.0


class DocumentOverviewResponse(BaseModel):
    """One window of page summaries for thumbnail navigation.

    `pages` covers `[offset, offset + len(pages))`. `total_pages` is the
    document length. A client pages with `offset` instead of requesting
    every page in one call.
    """
    document_id: str
    filename: str
    total_pages: int
    offset: int = 0
    limit: int = 24
    pages: List[PageOverviewItem]


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


class CreateFormFieldRequest(BaseModel):
    """Payload to create and place a new interactive form field on a page."""
    name: str
    field_type: str = Field(default="Text", description="Field type: Text, Checkbox, RadioButton, PushButton, Choice, Signature")
    min_x: float = Field(default=72.0)
    min_y: float = Field(default=700.0)
    max_x: float = Field(default=250.0)
    max_y: float = Field(default=724.0)
    value: Optional[str] = ""
    default_value: Optional[str] = None
    alt_name: Optional[str] = None
    options: Optional[List[str]] = None
    is_read_only: bool = False
    is_required: bool = False
    is_multiline: bool = False
    max_length: Optional[int] = None
    font_size: Optional[float] = 12.0


class CreateFormFieldResponse(BaseModel):
    """Response returned upon successfully creating an AcroForm field."""
    status: str = "ok"
    document_id: str
    field: FormFieldModel


class DeleteFormFieldResponse(BaseModel):
    """Response returned upon deleting an AcroForm field."""
    status: str = "ok"
    document_id: str
    deleted: bool
    field_name: str


class UpdateFormFieldRequest(BaseModel):
    """Payload to update an existing form field's geometry or properties."""
    min_x: Optional[float] = None
    min_y: Optional[float] = None
    max_x: Optional[float] = None
    max_y: Optional[float] = None
    alt_name: Optional[str] = None
    is_read_only: Optional[bool] = None
    is_required: Optional[bool] = None
    is_multiline: Optional[bool] = None


class UpdateFormFieldResponse(BaseModel):
    """Response returned upon updating an AcroForm field."""
    status: str = "ok"
    document_id: str
    field: FormFieldModel
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


class RedactionRegionItem(BaseModel):
    """Spatial bounding box rectangle for redaction."""
    min_x: float
    min_y: float
    max_x: float
    max_y: float


class RedactRegionsRequest(BaseModel):
    """Request payload to redact specific rectangular areas on a page."""
    page_number: int = Field(default=1, description="Target 1-based page number")
    regions: List[RedactionRegionItem] = Field(description="Bounding boxes to redact")
    fill_color: Optional[List[float]] = Field(default=[0.0, 0.0, 0.0], description="RGB fill color [r, g, b]")
    overlay_text: Optional[str] = Field(default="[REDACTADO]", description="Overlay label centered on blackout box")
    text_color: Optional[List[float]] = Field(default=[1.0, 1.0, 1.0], description="RGB overlay text color [r, g, b]")
    font_size: Optional[float] = Field(default=None, description="Font size or auto-calculated if None")
    prune_annotations: Optional[bool] = Field(default=True, description="Remove intersecting link and markup annotations")


class RedactPatternRequest(BaseModel):
    """Request payload to scan and redact sensitive PII patterns across pages."""
    pattern_type: str = Field(default="email", description="email, phone, ssn, credit_card, rfc, curp, text")
    custom_query: Optional[str] = Field(default=None, description="Search query if pattern_type is text")
    case_sensitive: Optional[bool] = Field(default=False, description="Case-sensitive matching")
    page_numbers: Optional[List[int]] = Field(default=None, description="1-based page numbers or null for all")
    fill_color: Optional[List[float]] = Field(default=[0.0, 0.0, 0.0], description="RGB fill color [r, g, b]")
    overlay_text: Optional[str] = Field(default="[REDACTADO]", description="Overlay label centered on blackout box")
    text_color: Optional[List[float]] = Field(default=[1.0, 1.0, 1.0], description="RGB overlay text color [r, g, b]")
    font_size: Optional[float] = Field(default=None, description="Font size or auto-calculated if None")
    prune_annotations: Optional[bool] = Field(default=True, description="Remove intersecting link and markup annotations")
    scrub_metadata: Optional[bool] = Field(default=False, description="Scrub Info dictionary and XMP metadata")


class RedactTextRequest(BaseModel):
    """Request payload to redact exact text occurrences."""
    query: str = Field(description="Search text to excise and redact")
    case_sensitive: Optional[bool] = Field(default=False, description="Case-sensitive search")
    page_numbers: Optional[List[int]] = Field(default=None, description="1-based page numbers or null for all")
    fill_color: Optional[List[float]] = Field(default=[0.0, 0.0, 0.0], description="RGB fill color [r, g, b]")
    overlay_text: Optional[str] = Field(default="[REDACTADO]", description="Overlay label centered on blackout box")
    text_color: Optional[List[float]] = Field(default=[1.0, 1.0, 1.0], description="RGB overlay text color [r, g, b]")
    font_size: Optional[float] = Field(default=None, description="Font size or auto-calculated if None")
    prune_annotations: Optional[bool] = Field(default=True, description="Remove intersecting link and markup annotations")


class SanitizeDocumentRequest(BaseModel):
    """Request payload to purge sensitive metadata from the document."""
    scrub_metadata: Optional[bool] = Field(default=True, description="Wipe Author, Title, Creator, and XMP Metadata")


class RedactionSummaryModel(BaseModel):
    """Summary of redaction results per page."""
    page_number: int
    purged_glyphs_count: int
    modified_blocks_count: int
    blackout_boxes_count: int
    pruned_annotations_count: int
    applied_rects: List[List[float]]


class RedactionActionResponse(BaseModel):
    """Response returned upon applying redactions."""
    success: bool
    document_id: str
    total_purged_glyphs: int
    total_blackout_boxes: int
    total_pruned_annotations: int
    summaries: List[RedactionSummaryModel]
    message: str


class SanitizeDocumentResponse(BaseModel):
    """Response returned upon sanitizing document metadata."""
    success: bool
    document_id: str
    modified: bool
    message: str


class PermissionsModel(BaseModel):
    """Bits stored in the encryption dictionary `/P` entry. This process does not enforce them."""
    print_low_res: bool = True
    print_high_res: bool = True
    modify_contents: bool = True
    copy_extract: bool = True
    modify_annotations: bool = True
    fill_forms: bool = True
    accessibility_extract: bool = True
    assemble_document: bool = True


class EncryptDocumentRequest(BaseModel):
    """Request payload to encrypt document with AES-128 and password protection."""
    user_password: str = Field(default="", description="Password required to open and read document")
    owner_password: str = Field(min_length=1, description="Owner password required to change permissions. No default is applied.")
    permissions: Optional[PermissionsModel] = Field(default=None, description="Bits stored in /P. This process does not enforce them.")
    encrypt_metadata: Optional[bool] = Field(default=True, description="Whether to encrypt metadata stream")


class DecryptDocumentRequest(BaseModel):
    """Request payload to remove encryption from a document."""
    password: str = Field(description="User or Owner password to decrypt the document")


class SignDocumentRequest(BaseModel):
    """Request payload to stamp a SHA-256 byte-range integrity attestation."""
    signer_name: str = Field(default="PDFEngine Certified Signer", description="Name stored in /Name. This does not prove identity.")
    reason: str = Field(default="Integridad del archivo", description="Note stored in /Reason")
    location: str = Field(default="Ciudad de México, MX", description="Physical or corporate signing location")
    page_number: int = Field(default=1, description="1-based page number where signature badge will appear")
    rect: Optional[List[float]] = Field(default=None, description="[min_x, min_y, max_x, max_y] bounding box or default placement")
    contact_info: Optional[str] = Field(default=None, description="Optional contact email or URL")


class SignatureModel(BaseModel):
    """Signature dictionary plus the SHA-256 byte-range check. contents_hex is the digest, not CMS."""
    field_name: str
    signer_name: str
    reason: str
    location: str
    date: str
    sub_filter: str
    byte_range: List[int]
    contents_hex: str
    byte_range_valid: bool
    rect: List[float]
    page_number: int


class SecurityStatusResponse(BaseModel):
    """Response detailing encryption and byte-range attestation status."""
    document_id: str
    is_encrypted: bool
    signatures: List[SignatureModel]


class SecurityActionResponse(BaseModel):
    """Generic action response for security operations."""
    success: bool
    document_id: str
    message: str
    is_encrypted: Optional[bool] = None
    signature: Optional[SignatureModel] = None


class TableCellModel(BaseModel):
    """A single cell inside an extracted table."""
    row: int
    col: int
    row_span: int = 1
    col_span: int = 1
    text: str
    is_header: bool = False
    bbox: BoundingBox


class TableModel(BaseModel):
    """A structured table extracted from a page."""
    table_idx: int
    page_number: int
    row_count: int
    col_count: int
    bbox: BoundingBox
    headers: List[str]
    rows: List[List[str]]
    cells: List[TableCellModel]


class PageTablesResponse(BaseModel):
    """Response containing all tables detected on a page."""
    document_id: str
    page_number: int
    total_tables: int
    tables: List[TableModel]


class TableExportResponse(BaseModel):
    """Response containing the formatted export content of a table."""
    document_id: str
    page_number: int
    table_idx: int
    format: str
    content: str
    row_count: int
    col_count: int


class OptimizeRequest(BaseModel):
    """Request payload to optimize document size and compress streams."""
    remove_unused: bool = Field(default=True, description="Eliminate unreferenced orphaned objects (Garbage Collection)")
    pack_object_streams: bool = Field(default=True, description="Pack indirect objects into compressed /ObjStm streams (PDF 1.5+)")
    recompress_flate: bool = Field(default=True, description="Recompress streams using optimal Flate (Zlib Best)")
    deduplicate_streams: bool = Field(default=True, description="Deduplicate identical streams and consolidate references")
    max_objects_per_stream: int = Field(default=100, description="Max indirect objects per /ObjStm container")


class OptimizeResponse(BaseModel):
    """Response containing optimization results and quantitative metrics."""
    success: bool
    document_id: str
    original_size: int
    optimized_size: int
    bytes_saved: int
    compression_ratio_pct: float
    objects_removed: int
    streams_recompressed: int
    object_streams_created: int
    streams_deduplicated: int
    message: str



