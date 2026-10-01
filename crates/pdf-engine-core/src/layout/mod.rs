//! Semantic layout reconstruction from visual glyph runs.
//!
//! Clusters positioned glyphs into spans, text lines, and paragraph blocks
//! with deterministic alignment, leading, and bounding box computation.

pub mod geometry;
pub mod glyph;
pub mod line;
pub mod paragraph;
pub mod reconstructor;
pub mod span;

pub use geometry::{Point, Rect};
pub use glyph::PositionedGlyph;
pub use line::TextLine;
pub use paragraph::{ParagraphBlock, TextAlignment};
pub use reconstructor::LayoutReconstructor;
pub use span::TextSpan;
