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

pub mod cos;
pub mod editor;
pub mod error;
pub mod fonts;
pub mod layout;
pub mod security;
pub mod stream;

// Re-export primary types for ergonomic usage
pub use cos::{ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, PdfString};
pub use editor::{ReflowEngine, ReflowLine, SurgicalEditor};
pub use error::{PdfError, PdfResult};
pub use fonts::{compose_ligatures, decompose_ligatures, FontMetrics, ToUnicodeMap};
pub use layout::{
    LayoutReconstructor, ParagraphBlock, Point, PositionedGlyph, Rect, TextAlignment, TextLine,
    TextSpan,
};
pub use security::SecurityLimits;
pub use stream::{ContentAst, ContentNode, ContentParser, GraphicsStateStack, Matrix, Operation};
