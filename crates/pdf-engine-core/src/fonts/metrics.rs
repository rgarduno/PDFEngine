//! ISO 32000-1 §9.6.2.2 Font Metrics, Character Advances, and Kerning.
//!
//! Provides deterministic advance width calculations using the font's `/Widths` array,
//! `/FontDescriptor` metrics, character spacing ($T_c$), word spacing ($T_w$),
//! and horizontal kerning adjustments from `TJ` arrays.

use std::collections::BTreeMap;

use crate::stream::graphics_state::TextState;

/// Font metrics specifying character advance widths and baseline geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct FontMetrics {
    /// First character code defined in `/Widths` array (ISO 32000-1 §9.6.2.2).
    pub first_char: u32,
    /// Last character code defined in `/Widths` array.
    pub last_char: u32,
    /// Glyphs advance widths in 1/1000 units of text space (for simple 8-bit fonts).
    pub widths: Vec<f64>,
    /// Sparse CID glyph advance widths (for composite Type0/CID fonts, ISO 32000-1 §9.7.4.3).
    pub cid_widths: Option<BTreeMap<u32, f64>>,
    /// Fallback width used for glyphs not present in `/Widths` (from `/MissingWidth` or `/DW`).
    pub default_width: f64,
    /// Ascender height in 1/1000 units above baseline.
    pub ascent: f64,
    /// Descender depth in 1/1000 units below baseline (typically negative).
    pub descent: f64,
}

impl Default for FontMetrics {
    fn default() -> Self {
        Self {
            first_char: 0,
            last_char: 255,
            widths: Vec::new(),
            cid_widths: None,
            default_width: 1000.0,
            ascent: 750.0,
            descent: -250.0,
        }
    }
}

impl FontMetrics {
    /// Creates a new font metrics descriptor for a simple 8-bit font.
    pub fn new(first_char: u32, last_char: u32, widths: Vec<f64>, default_width: f64) -> Self {
        Self {
            first_char,
            last_char,
            widths,
            cid_widths: None,
            default_width,
            ascent: 750.0,
            descent: -250.0,
        }
    }

    /// Creates a new font metrics descriptor for a composite Type0/CIDFont.
    pub fn new_cid(cid_widths: BTreeMap<u32, f64>, default_width: f64) -> Self {
        Self {
            first_char: 0,
            last_char: 65535,
            widths: Vec::new(),
            cid_widths: Some(cid_widths),
            default_width,
            ascent: 750.0,
            descent: -250.0,
        }
    }

    /// Looks up the unscaled glyph advance width (in 1/1000 of a text unit) for a character code.
    pub fn get_glyph_width(&self, char_code: u32) -> f64 {
        if let Some(cids) = &self.cid_widths {
            if let Some(&w) = cids.get(&char_code) {
                return w;
            }
            return self.default_width;
        }
        if char_code >= self.first_char && char_code <= self.last_char {
            let index = (char_code - self.first_char) as usize;
            if index < self.widths.len() {
                return self.widths[index];
            }
        }
        self.default_width
    }

    /// Computes the exact horizontal advance displacement for a single character code
    /// taking into account font size ($T_{fs}$), character spacing ($T_c$), word spacing ($T_w$),
    /// and horizontal scaling ($T_h$) according to ISO 32000-1 §9.4.4.
    ///
    /// $$\Delta x = \left( \left( \frac{w_0}{1000} \times T_{fs} \right) + T_c + \begin{cases} T_w & \text{if space} \\ 0 & \text{otherwise} \end{cases} \right) \times \frac{T_h}{100}$$
    pub fn compute_char_advance(&self, char_code: u32, text_state: &TextState) -> f64 {
        let w0 = self.get_glyph_width(char_code);
        let mut advance = (w0 / 1000.0) * text_state.font_size + text_state.char_spacing;

        // Word spacing Tw applies to ASCII space (code 32)
        if char_code == 32 {
            advance += text_state.word_spacing;
        }

        advance * (text_state.horizontal_scaling / 100.0)
    }

    /// Computes the horizontal displacement delta resulting from a numeric kerning adjustment
    /// within a `TJ` array operand (ISO 32000-1 §9.4.3).
    ///
    /// In PDF `TJ` arrays, negative numbers move the cursor forward (spacing out letters),
    /// while positive numbers move the cursor backward (tightening kerning):
    /// $$\Delta x_{kerning} = -\frac{k}{1000} \times T_{fs} \times \frac{T_h}{100}$$
    pub fn compute_kerning_displacement(kerning_num: f64, text_state: &TextState) -> f64 {
        -(kerning_num / 1000.0) * text_state.font_size * (text_state.horizontal_scaling / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_advance_and_kerning_calculation() {
        let metrics = FontMetrics::new(65, 66, vec![600.0, 700.0], 500.0);
        let mut state = TextState::default();
        state.font_size = 12.0;
        state.char_spacing = 0.5;
        state.horizontal_scaling = 100.0;

        // Char 'A' (code 65), width = 600
        // (600 / 1000 * 12) + 0.5 = 7.2 + 0.5 = 7.7
        let advance_a = metrics.compute_char_advance(65, &state);
        assert!((advance_a - 7.7).abs() < 1e-6);

        // Kerning displacement: k = -100
        // -(-100 / 1000) * 12 = 1.2
        let kerning_dx = FontMetrics::compute_kerning_displacement(-100.0, &state);
        assert!((kerning_dx - 1.2).abs() < 1e-6);
    }
}
