//! Typographic reflow and line breaking engine.
//!
//! Re-flows modified text into paragraph lines adhering to bounding box constraints,
//! font metrics, and alignment rules (left, center, right, justified).

use crate::fonts::{FontEncoder, FontMetrics};
use crate::layout::paragraph::TextAlignment;
use crate::stream::graphics_state::TextState;

/// A single re-flowed line of text with its computed width and horizontal offset.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflowLine {
    /// Text content of this line.
    pub text: String,
    /// Computed width of the line in points.
    pub width: f64,
    /// Horizontal offset $x$ relative to the left margin of the paragraph bounding box.
    pub offset_x: f64,
    /// Vertical baseline offset $y$ relative to the top of the paragraph.
    pub offset_y: f64,
    /// Word spacing adjustment ($T_w$) for justified lines in points.
    pub word_spacing: f64,
}

/// Reflow engine computing line breaks and alignment adjustments.
pub struct ReflowEngine;

impl ReflowEngine {
    /// Breaks and re-flows text within a bounding box width constraint.
    pub fn reflow(
        text: &str,
        max_width: f64,
        metrics: &FontMetrics,
        text_state: &TextState,
        alignment: TextAlignment,
        leading: f64,
    ) -> Vec<ReflowLine> {
        let paragraphs: Vec<&str> = text.split('\n').collect();
        let mut result_lines = Vec::new();
        let mut cur_y = 0.0;

        for para in paragraphs {
            let words: Vec<&str> = para.split_whitespace().collect();
            if words.is_empty() {
                // Empty line
                cur_y -= leading;
                continue;
            }

            let mut cur_line_words: Vec<&str> = Vec::new();
            let mut cur_line_width = 0.0;
            let space_advance = metrics.compute_char_advance(32, text_state);

            for word in words {
                let word_width = Self::compute_word_width(word, metrics, text_state);

                let needed_width = if cur_line_words.is_empty() {
                    word_width
                } else {
                    cur_line_width + space_advance + word_width
                };

                if needed_width <= max_width || cur_line_words.is_empty() {
                    cur_line_words.push(word);
                    cur_line_width = needed_width;
                } else {
                    // Line break
                    let is_last_line = false;
                    let line = Self::format_line(
                        &cur_line_words,
                        cur_line_width,
                        max_width,
                        cur_y,
                        alignment,
                        is_last_line,
                    );
                    result_lines.push(line);
                    cur_y -= leading;

                    cur_line_words = vec![word];
                    cur_line_width = word_width;
                }
            }

            if !cur_line_words.is_empty() {
                let is_last_line = true;
                let line = Self::format_line(
                    &cur_line_words,
                    cur_line_width,
                    max_width,
                    cur_y,
                    alignment,
                    is_last_line,
                );
                result_lines.push(line);
                cur_y -= leading;
            }
        }

        result_lines
    }

    /// Computes advance width of an individual word in points.
    fn compute_word_width(word: &str, metrics: &FontMetrics, text_state: &TextState) -> f64 {
        let mut width = 0.0;
        let encoder = FontEncoder::new();
        for c in word.chars() {
            let (code, _) = encoder.encode_char(c);
            width += metrics.compute_char_advance(code, text_state);
        }
        width
    }

    /// Formats words into a ReflowLine with alignment offset and justification spacing.
    fn format_line(
        words: &[&str],
        content_width: f64,
        max_width: f64,
        offset_y: f64,
        alignment: TextAlignment,
        is_last_line: bool,
    ) -> ReflowLine {
        let text = words.join(" ");

        let (offset_x, word_spacing) = match alignment {
            TextAlignment::Left => (0.0, 0.0),
            TextAlignment::Center => {
                let slack = (max_width - content_width).max(0.0);
                (slack * 0.5, 0.0)
            }
            TextAlignment::Right => {
                let slack = (max_width - content_width).max(0.0);
                (slack, 0.0)
            }
            TextAlignment::Justified => {
                if is_last_line || words.len() <= 1 {
                    (0.0, 0.0)
                } else {
                    let slack = (max_width - content_width).max(0.0);
                    let spaces_count = words.len() - 1;
                    let extra_tw = slack / spaces_count as f64;
                    (0.0, extra_tw)
                }
            }
        };

        ReflowLine {
            text,
            width: content_width,
            offset_x,
            offset_y,
            word_spacing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflow_greedy_line_breaking() {
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
        let mut state = TextState::default();
        state.font_size = 10.0;

        // Each char = 5 points, space = 5 points
        // "Hello World PDF Engine"
        // "Hello" = 25 pt, " " = 5 pt, "World" = 25 pt -> total = 55 pt
        // With max_width = 60, "Hello World" fits on line 1, "PDF Engine" on line 2
        let text = "Hello World PDF Engine";
        let lines = ReflowEngine::reflow(text, 60.0, &metrics, &state, TextAlignment::Left, 12.0);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello World");
        assert_eq!(lines[1].text, "PDF Engine");
    }
}
