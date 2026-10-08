//! Positioned glyph primitives extracted from content streams.
//!
//! Represents a single rendered character with exact page device coordinates,
//! bounding box, typographic font size, and source AST tracking.

use crate::layout::geometry::{Point, Rect};
use crate::stream::ast::NodeId;

/// A single visually positioned glyph on a PDF page.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionedGlyph {
    /// Character code in the PDF font encoding / CID.
    pub char_code: u32,
    /// Canonical Unicode string representation (from /ToUnicode or ASCII fallback).
    pub unicode: String,
    /// Origin coordinate of the glyph on the text baseline in page space.
    pub origin: Point,
    /// Advance width displacement in page points.
    pub advance: f64,
    /// Bounding box enclosing the rendered glyph in page space.
    pub bbox: Rect,
    /// Active font identifier resource name (e.g. `/F1`).
    pub font_name: String,
    /// Font size operand of `Tf`, in text space. Surgery writes this value back.
    pub font_size: f64,
    /// Visual size in page points: the length of the text rendering matrix's y basis.
    pub rendered_size: f64,
    /// Source AST node ID in the content stream where this glyph originated.
    pub ast_node_id: NodeId,
}

impl PositionedGlyph {
    /// Creates a new positioned glyph.
    pub fn new(
        char_code: u32,
        unicode: impl Into<String>,
        origin: Point,
        advance: f64,
        font_name: impl Into<String>,
        font_size: f64,
        rendered_size: f64,
        ast_node_id: NodeId,
    ) -> Self {
        let height = if rendered_size.abs() < 1e-6 {
            font_size.abs()
        } else {
            rendered_size.abs()
        };
        let bbox = Rect::from_origin_size(origin.x, origin.y, advance, height);
        Self {
            char_code,
            unicode: unicode.into(),
            origin,
            advance,
            bbox,
            font_name: font_name.into(),
            font_size,
            rendered_size,
            ast_node_id,
        }
    }
}
