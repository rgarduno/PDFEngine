//! # PDFEngine Core
//!
//! High-performance, memory-safe, lossless PDF parsing and surgical editing engine
//! compliant with ISO 32000-1 and ISO 32000-2.
//!
//! ## Architectural Layers
//!
//! 1. **Carousel Object System (`cos`)**: Lexical scanning, syntactic parsing, indirect
//!    object indexing (`XRef`), compression filters (`FlateDecode`), and deterministic writing.
//! 2. **Content Stream Engine (`stream`)**: Lossless AST decomposition of visual display lists
//!    and graphics state matrix projection ($CTM \times T_m$).
//! 3. **Typographic Engine (`fonts`)**: OpenType/TrueType parsing, `/ToUnicode` CMaps,
//!    and ligature/font fallback metrics.
//! 4. **Layout Reconstruction (`layout`)**: Spatial clustering from glyphs to paragraphs.
//! 5. **Surgical Editor (`editor`)**: Atomic in-place reflow and stream mutation.
//! 6. **Security Hardening (`security`)**: Bounded memory limits, recursion caps,
//!    decompression guards, and active-content removal on save.
#![allow(
    clippy::field_reassign_with_default,
    clippy::vec_init_then_push,
    clippy::manual_repeat_n,
    clippy::manual_range_contains,
    clippy::explicit_auto_deref,
    clippy::chunks_exact_to_as_chunks,
    clippy::large_enum_variant,
    clippy::needless_lifetimes,
    clippy::manual_is_multiple_of,
    clippy::ptr_arg,
    clippy::derivable_impls,
    clippy::collapsible_match,
    clippy::unnecessary_lazy_evaluations,
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::too_many_arguments,
    clippy::get_first,
    clippy::should_implement_trait,
    clippy::op_ref,
    clippy::type_complexity,
    clippy::manual_rem_euclid,
    clippy::unnecessary_sort_by
)]

pub mod annots;
pub mod cos;
pub mod crypto;
pub mod editor;
pub mod error;
pub mod fonts;
pub mod forms;
pub mod images;
pub mod layout;
pub mod ocr;
pub mod ops;
pub mod pdfa;
pub mod redact;
pub mod security;
pub mod stream;
pub mod tables;
pub mod watermark;

// Re-export primary types for ergonomic usage
pub use annots::{
    add_link_goto, add_link_uri, add_stamp, add_text_markup, delete_annotation,
    extract_all_annotations, extract_page_annotations, flatten_annotations, Annotation,
    AnnotationSubtype, LinkAction, StampType,
};
pub use cos::{
    ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, PdfString,
};
pub use editor::{ReflowEngine, ReflowLine, SurgicalEditor};
pub use error::{PdfError, PdfResult};
pub use fonts::{
    compose_ligatures, css_face, decompose_ligatures, resolve_page_fonts, FontEncoder, FontMetrics,
    GlyphFallback, ResolvedFont, ToUnicodeMap, WinAnsiEncoding,
};
pub use forms::{
    extract_document_forms, fill_field_value, fill_fields_batch, flatten_document_forms, FormField,
    FormFieldType,
};
pub use images::{
    create_image_xobject, encode_png, extract_page_images, get_image_binary, parse_jpeg,
    parse_png_header, parse_png_pixels, replace_image_content, ImageInfo, JpegHeader, PngHeader,
};
pub use layout::{
    LayoutReconstructor, ParagraphBlock, Point, PositionedGlyph, Rect, TextAlignment, TextLine,
    TextSpan,
};
pub use ocr::{add_searchable_text_layer, OcrReport};
pub use ops::{
    collect_garbage, compare_documents, compute_word_diffs, deduplicate_streams, delete_pages,
    extract_metadata, extract_pages, get_page_rotation, merge_documents, merge_pdf_bytes,
    optimize_document, recompress_streams, reorder_pages, rotate_all_pages, rotate_page,
    save_optimized_to_vec, set_page_rotation, split_by_ranges, split_document, update_metadata,
    DiffKind, DiffOptions, DiffReport, DiffSummary, DocumentMetadata, ImageDiffItem,
    MetadataDiffItem, ObjectCloner, OptimizationOptions, OptimizationStats, PageDiff,
    PageDimensions, TextDiffItem, WordDiff,
};
pub use pdfa::{convert_to_pdfa, validate_pdfa, PdfALevel};
pub use redact::{
    apply_redaction_to_ast, find_credit_cards, find_curp, find_emails, find_matches,
    find_pattern_boxes_on_page, find_phones, find_rfc, find_ssn, find_substring,
    prune_page_annotations, redact_document_pattern, redact_document_rectangles, redact_page,
    scrub_document_metadata, RedactionConfig, RedactionPattern, RedactionRect, RedactionSummary,
};
pub use security::SecurityLimits;
pub use stream::{ContentAst, ContentNode, ContentParser, GraphicsStateStack, Matrix, Operation};
pub use tables::{
    detect_borderless_tables, detect_lattice_tables, detect_tables, export_table, export_to_csv,
    export_to_html, export_to_json, export_to_markdown, DetectedTable, TableCell,
    TableExportFormat,
};
pub use watermark::{
    apply_image_watermark, apply_pagination, apply_text_watermark, ImageWatermarkConfig,
    PaginationConfig, PaginationPosition, TextWatermarkConfig, WatermarkPlacement,
};
