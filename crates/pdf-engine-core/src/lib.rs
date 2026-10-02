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
//!    and decompression bomb guards.

pub mod annots;
pub mod cos;
pub mod editor;
pub mod error;
pub mod fonts;
pub mod forms;
pub mod images;
pub mod layout;
pub mod ops;
pub mod security;
pub mod stream;

// Re-export primary types for ergonomic usage
pub use cos::{ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, PdfString};
pub use editor::{ReflowEngine, ReflowLine, SurgicalEditor};
pub use error::{PdfError, PdfResult};
pub use fonts::{
    compose_ligatures, decompose_ligatures, FontEncoder, FontMetrics, GlyphFallback, ToUnicodeMap,
    WinAnsiEncoding,
};
pub use forms::{
    extract_document_forms, fill_field_value, fill_fields_batch, flatten_document_forms, FormField,
    FormFieldType,
};
pub use images::{
    encode_png, extract_page_images, get_image_binary, parse_jpeg, parse_png_header,
    parse_png_pixels, replace_image_content, ImageInfo, JpegHeader, PngHeader,
};
pub use ops::{
    delete_pages, extract_pages, get_page_rotation, merge_documents, merge_pdf_bytes,
    reorder_pages, rotate_all_pages, rotate_page, set_page_rotation, split_by_ranges,
    split_document, ObjectCloner,
};
pub use layout::{
    LayoutReconstructor, ParagraphBlock, Point, PositionedGlyph, Rect, TextAlignment, TextLine,
    TextSpan,
};
pub use security::SecurityLimits;
pub use stream::{ContentAst, ContentNode, ContentParser, GraphicsStateStack, Matrix, Operation};
pub use annots::{
    add_link_goto, add_link_uri, add_stamp, add_text_markup, delete_annotation,
    extract_all_annotations, extract_page_annotations, flatten_annotations, Annotation,
    AnnotationSubtype, LinkAction, StampType,
};
