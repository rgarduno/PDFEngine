//! TextSpan grouping contiguous glyphs sharing common typographic style.

use crate::layout::geometry::Rect;
use crate::layout::glyph::PositionedGlyph;

/// A contiguous run of glyphs sharing the same font, size, and baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    /// Combined text content of the span.
    pub text: String,
    /// Bounding box enclosing all glyphs in this span.
    pub bbox: Rect,
    /// Active font identifier resource name (e.g. `/F1`).
    pub font_name: String,
    /// Active font size operand of `Tf`, in text space.
    pub font_size: f64,
    /// Visual size in page points, copied from the first glyph.
    pub rendered_size: f64,
    /// Baseline y-coordinate in page space.
    pub baseline_y: f64,
    /// The individual positioned glyphs forming this span.
    pub glyphs: Vec<PositionedGlyph>,
}

impl TextSpan {
    /// Constructs a span from a non-empty sequence of contiguous glyphs.
    pub fn from_glyphs(glyphs: Vec<PositionedGlyph>) -> Option<Self> {
        if glyphs.is_empty() {
            return None;
        }

        let first = &glyphs[0];
        let font_name = first.font_name.clone();
        let font_size = first.font_size;
        let rendered_size = first.rendered_size;
        let baseline_y = first.origin.y;

        let mut text = String::new();
        let mut bbox = first.bbox;

        for g in &glyphs {
            text.push_str(&g.unicode);
            bbox = bbox.union(&g.bbox);
        }

        Some(Self {
            text,
            bbox,
            font_name,
            font_size,
            rendered_size,
            baseline_y,
            glyphs,
        })
    }
}
