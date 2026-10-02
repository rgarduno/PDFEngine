//! ISO 32000-1 Annex D Character Encodings, WinAnsi, and Dynamic Glyph Fallback.
//!
//! Handles character code translation between Unicode strings and single-byte or CID font
//! encodings, providing metric-compatible fallback and synthetic transliteration for
//! characters missing from embedded font subsets.

use std::collections::HashMap;
use crate::fonts::tounicode::ToUnicodeMap;

/// Standard WinAnsiEncoding translation table (ISO 32000-1 Annex D.2).
pub struct WinAnsiEncoding;

impl WinAnsiEncoding {
    /// Maps a Unicode character to its 8-bit WinAnsi character code if supported.
    pub fn unicode_to_code(c: char) -> Option<u8> {
        let u = c as u32;

        // Standard ASCII range (0x20 ..= 0x7E)
        if (0x20..=0x7E).contains(&u) {
            return Some(u as u8);
        }

        // Direct Latin-1 Supplement range (0xA0 ..= 0xFF)
        if (0xA0..=0xFF).contains(&u) {
            return Some(u as u8);
        }

        // Windows-1252 extensions in the 0x80 ..= 0x9F range
        match c {
            '€' => Some(0x80),
            '‚' => Some(0x82),
            'ƒ' => Some(0x83),
            '„' => Some(0x84),
            '…' => Some(0x85),
            '†' => Some(0x86),
            '‡' => Some(0x87),
            'ˆ' => Some(0x88),
            '‰' => Some(0x89),
            'Š' => Some(0x8A),
            '‹' => Some(0x8B),
            'Œ' => Some(0x8C),
            'Ž' => Some(0x8E),
            '‘' => Some(0x91),
            '’' => Some(0x92),
            '“' => Some(0x93),
            '”' => Some(0x94),
            '•' => Some(0x95),
            '–' => Some(0x96),
            '—' => Some(0x97),
            '˜' => Some(0x98),
            '™' => Some(0x99),
            'š' => Some(0x9A),
            '›' => Some(0x9B),
            'œ' => Some(0x9C),
            'ž' => Some(0x9E),
            'Ÿ' => Some(0x9F),
            _ => None,
        }
    }

    /// Maps an 8-bit WinAnsi character code to its Unicode representation.
    pub fn code_to_unicode(code: u8) -> char {
        if (0x20..=0x7E).contains(&code) || (0xA0..=0xFF).contains(&code) {
            return code as char;
        }

        match code {
            0x80 => '€',
            0x82 => '‚',
            0x83 => 'ƒ',
            0x84 => '„',
            0x85 => '…',
            0x86 => '†',
            0x87 => '‡',
            0x88 => 'ˆ',
            0x89 => '‰',
            0x8A => 'Š',
            0x8B => '‹',
            0x8C => 'Œ',
            0x8E => 'Ž',
            0x91 => '‘',
            0x92 => '’',
            0x93 => '“',
            0x94 => '”',
            0x95 => '•',
            0x96 => '–',
            0x97 => '—',
            0x98 => '˜',
            0x99 => '™',
            0x9A => 'š',
            0x9B => '›',
            0x9C => 'œ',
            0x9E => 'ž',
            0x9F => 'Ÿ',
            _ => ' ',
        }
    }
}

/// Fallback transliteration mapping for glyphs not present in font subsets.
pub struct GlyphFallback;

impl GlyphFallback {
    /// Returns a metric-compatible ASCII fallback character for accented or special glyphs.
    pub fn transliterate(c: char) -> char {
        match c {
            'á' | 'à' | 'â' | 'ã' | 'å' | 'ā' => 'a',
            'Á' | 'À' | 'Â' | 'Ã' | 'Å' | 'Ā' => 'A',
            'é' | 'è' | 'ê' | 'ë' | 'ē' => 'e',
            'É' | 'È' | 'Ê' | 'Ë' | 'Ē' => 'E',
            'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
            'Í' | 'Ì' | 'Î' | 'Ï' | 'Ī' => 'I',
            'ó' | 'ò' | 'ô' | 'õ' | 'ō' => 'o',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ō' => 'O',
            'ú' | 'ù' | 'û' | 'ü' | 'ū' => 'u',
            'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ū' => 'U',
            'ñ' => 'n',
            'Ñ' => 'N',
            'ç' => 'c',
            'Ç' => 'C',
            'ß' => 's',
            '¿' => '?',
            '¡' => '!',
            '—' | '–' => '-',
            '“' | '”' => '"',
            '‘' | '’' => '\'',
            '…' => '.',
            _ => '?',
        }
    }
}

/// Dynamic font encoder with fallback glyph injection for subsetted fonts.
#[derive(Debug, Clone, Default)]
pub struct FontEncoder {
    /// Optional `/ToUnicode` CMap reverse mapping.
    pub to_unicode: Option<ToUnicodeMap>,
    /// Set of known supported character codes.
    pub supported_codes: HashMap<char, u32>,
}

impl FontEncoder {
    /// Creates a new encoder with standard WinAnsi capability.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches a `/ToUnicode` map for reverse CID/code resolution.
    pub fn with_tounicode(mut self, cmap: ToUnicodeMap) -> Self {
        for (code, s) in &cmap.char_to_unicode {
            if let Some(first_char) = s.chars().next() {
                self.supported_codes.insert(first_char, *code);
            }
        }
        self.to_unicode = Some(cmap);
        self
    }

    /// Determines if a character is directly representable in the active font.
    pub fn is_glyph_available(&self, c: char) -> bool {
        if self.supported_codes.contains_key(&c) {
            return true;
        }
        if let Some(cmap) = &self.to_unicode {
            if cmap.encode_char(&c.to_string()).is_some() {
                return true;
            }
        }
        WinAnsiEncoding::unicode_to_code(c).is_some()
    }

    /// Encodes a Unicode character into a font character code, applying fallback if missing.
    pub fn encode_char(&self, c: char) -> (u32, bool) {
        // 1. Direct lookup in supported codes from ToUnicode CMap
        if let Some(&code) = self.supported_codes.get(&c) {
            return (code, false);
        }

        // 2. Lookup in ToUnicode reverse map
        if let Some(cmap) = &self.to_unicode {
            if let Some(code) = cmap.encode_char(&c.to_string()) {
                return (code, false);
            }
        }

        // 3. Lookup in standard WinAnsiEncoding
        if let Some(code) = WinAnsiEncoding::unicode_to_code(c) {
            return (code as u32, false);
        }

        // 4. Missing glyph: apply metric-compatible transliteration fallback
        let fallback_char = GlyphFallback::transliterate(c);
        if let Some(code) = WinAnsiEncoding::unicode_to_code(fallback_char) {
            (code as u32, true)
        } else {
            (b'?' as u32, true)
        }
    }

    /// Encodes a full Unicode string into raw PDF content stream literal bytes.
    pub fn encode_string(&self, text: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(text.len());
        for c in text.chars() {
            let (code, _) = self.encode_char(c);
            if code <= 0xFF {
                bytes.push(code as u8);
            } else {
                // 16-bit CID composite encoding
                bytes.push((code >> 8) as u8);
                bytes.push((code & 0xFF) as u8);
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_winansi_accents_and_symbols() {
        assert_eq!(WinAnsiEncoding::unicode_to_code('ñ'), Some(0xF1));
        assert_eq!(WinAnsiEncoding::unicode_to_code('á'), Some(0xE1));
        assert_eq!(WinAnsiEncoding::unicode_to_code('€'), Some(0x80));
        assert_eq!(WinAnsiEncoding::unicode_to_code('¿'), Some(0xBF));
        assert_eq!(WinAnsiEncoding::unicode_to_code('¡'), Some(0xA1));

        assert_eq!(WinAnsiEncoding::code_to_unicode(0xF1), 'ñ');
        assert_eq!(WinAnsiEncoding::code_to_unicode(0xE1), 'á');
        assert_eq!(WinAnsiEncoding::code_to_unicode(0x80), '€');
    }

    #[test]
    fn test_encoder_glyph_availability_and_fallback() {
        let encoder = FontEncoder::new();

        // Standard ASCII
        assert!(encoder.is_glyph_available('A'));
        assert_eq!(encoder.encode_char('A'), (65, false));

        // Accented Spanish character available in WinAnsi
        assert!(encoder.is_glyph_available('ñ'));
        assert_eq!(encoder.encode_char('ñ'), (0xF1, false));

        // Text encoding roundtrip
        let encoded = encoder.encode_string("Contrato de Términos y Año: $500 €");
        assert!(encoded.contains(&0xF1)); // Contains 'ñ' code (0xF1)
        assert!(encoded.contains(&0xE9)); // Contains 'é' code (0xE9)
        assert!(encoded.contains(&0x80)); // Contains '€' code (0x80)
    }
}
