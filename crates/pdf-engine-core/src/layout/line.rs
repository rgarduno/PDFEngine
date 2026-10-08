//! TextLine representation clustering typographic spans on a common baseline.

use crate::layout::geometry::Rect;
use crate::layout::span::TextSpan;

/// A single horizontal line of text composed of one or more styled spans.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// Combined text string across all spans in this line.
    pub text: String,
    /// Bounding box enclosing the entire line.
    pub bbox: Rect,
    /// Baseline y-coordinate in page space.
    pub baseline_y: f64,
    /// Leftmost x-coordinate.
    pub start_x: f64,
    /// Rightmost x-coordinate.
    pub end_x: f64,
    /// The individual styled spans forming this line.
    pub spans: Vec<TextSpan>,
}

impl TextLine {
    /// Constructs a text line from an ordered vector of text spans.
    pub fn from_spans(mut spans: Vec<TextSpan>) -> Option<Self> {
        if spans.is_empty() {
            return None;
        }

        // Sort spans horizontally by start x
        spans.sort_by(|a, b| a.bbox.min_x.partial_cmp(&b.bbox.min_x).unwrap_or(std::cmp::Ordering::Equal));

        let baseline_y = spans[0].baseline_y;
        let mut bbox = spans[0].bbox;
        let mut text = String::new();

        for (i, span) in spans.iter().enumerate() {
            if i > 0 {
                // If there is a noticeable gap between spans and no trailing space, insert space
                let prev_end = spans[i - 1].bbox.max_x;
                let cur_start = span.bbox.min_x;
                let gap = cur_start - prev_end;
                let visual = span.rendered_size.max(span.font_size);
                let space_threshold = visual * 0.25;

                if gap > space_threshold && !text.ends_with(' ') && !span.text.starts_with(' ') {
                    text.push(' ');
                }
            }

            text.push_str(&span.text);
            bbox = bbox.union(&span.bbox);
        }

        let start_x = bbox.min_x;
        let end_x = bbox.max_x;

        Some(Self {
            text,
            bbox,
            baseline_y,
            start_x,
            end_x,
            spans,
        })
    }
}
