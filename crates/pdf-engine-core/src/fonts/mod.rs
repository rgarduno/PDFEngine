//! Font parsing, metrics calculation, and ToUnicode CMap decoding.
//!
//! Handles TrueType/OpenType tables, subsetting identification, and glyph injection
//! according to ISO 32000-1 §9.

pub mod ligatures;
pub mod metrics;
pub mod tounicode;
pub mod truetype;

pub use ligatures::{compose_ligatures, decompose_ligatures};
pub use metrics::FontMetrics;
pub use tounicode::ToUnicodeMap;
pub use truetype::SfntFont;
