//! Font parsing, metrics calculation, and ToUnicode CMap decoding.
//!
//! Handles TrueType/OpenType tables, subsetting identification, and glyph injection
//! according to ISO 32000-1 §9.

pub mod encoding;
pub mod ligatures;
pub mod metrics;
pub mod resolve;
pub mod tounicode;
pub mod truetype;

pub use encoding::{FontEncoder, GlyphFallback, WinAnsiEncoding};
pub use ligatures::{compose_ligatures, decompose_ligatures};
pub use metrics::FontMetrics;
pub use resolve::{css_face, resolve_page_fonts, ResolvedFont};
pub use tounicode::ToUnicodeMap;
pub use truetype::SfntFont;
