//! ParagraphBlock clustering lines into coherent editable paragraph units.

use crate::layout::geometry::Rect;
use crate::layout::line::TextLine;
use crate::stream::ast::NodeId;

/// Typographic alignment of a paragraph block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignment {
    /// Left aligned (flush left, ragged right).
    Left,
    /// Centered horizontally.
    Center,
    /// Right aligned (flush right, ragged left).
    Right,
    /// Justified (flush left and right across non-terminal lines).
    Justified,
}

/// A coherent block of paragraph lines with detected alignment, leading, and source mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphBlock {
    /// Unique identifier for this paragraph block on the page.
    pub id: usize,
    /// Ordered sequence of text lines in reading order (top to bottom).
    pub lines: Vec<TextLine>,
    /// Bounding box enclosing the entire paragraph block.
    pub bbox: Rect,
    /// Typographic alignment detected across the block's lines.
    pub alignment: TextAlignment,
    /// Average vertical leading (line-to-line baseline distance) in points.
    pub leading: f64,
    /// Set of content stream AST node IDs that contributed to this paragraph.
    pub source_node_ids: Vec<NodeId>,
}

impl ParagraphBlock {
    /// Constructs a paragraph block from an ordered vector of lines.
    pub fn new(id: usize, lines: Vec<TextLine>, source_node_ids: Vec<NodeId>) -> Option<Self> {
        if lines.is_empty() {
            return None;
        }

        let mut bbox = lines[0].bbox;
        let mut total_leading = 0.0;
        let mut leading_count = 0;

        for (i, line) in lines.iter().enumerate() {
            bbox = bbox.union(&line.bbox);
            if i > 0 {
                let dy = (lines[i - 1].baseline_y - line.baseline_y).abs();
                total_leading += dy;
                leading_count += 1;
            }
        }

        let leading = if leading_count > 0 {
            total_leading / leading_count as f64
        } else {
            lines[0].bbox.height() * 1.2
        };

        let alignment = Self::detect_alignment(&lines);

        Some(Self {
            id,
            lines,
            bbox,
            alignment,
            leading,
            source_node_ids,
        })
    }

    /// Combines all line texts into a single string separated by newlines.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<&str>>()
            .join("\n")
    }

    /// Detects typographic alignment across multiple lines according to margin variances.
    pub fn detect_alignment(lines: &[TextLine]) -> TextAlignment {
        if lines.len() <= 1 {
            return TextAlignment::Left;
        }

        let tolerance = 3.0; // 3 points tolerance for slight kerning / margin jitter

        let first_left = lines[0].bbox.min_x;
        let first_right = lines[0].bbox.max_x;

        let mut left_aligned = true;
        let mut right_aligned = true;
        let mut center_aligned = true;

        let first_center = lines[0].bbox.center_x();

        // Check non-terminal lines (as the last line of a paragraph is typically short)
        let check_count = if lines.len() > 2 {
            lines.len() - 1
        } else {
            lines.len()
        };

        for line in &lines[1..check_count] {
            if (line.bbox.min_x - first_left).abs() > tolerance {
                left_aligned = false;
            }
            if (line.bbox.max_x - first_right).abs() > tolerance {
                right_aligned = false;
            }
            if (line.bbox.center_x() - first_center).abs() > tolerance {
                center_aligned = false;
            }
        }

        if left_aligned && right_aligned {
            TextAlignment::Justified
        } else if center_aligned && !left_aligned {
            TextAlignment::Center
        } else if right_aligned && !left_aligned {
            TextAlignment::Right
        } else {
            TextAlignment::Left
        }
    }
}
