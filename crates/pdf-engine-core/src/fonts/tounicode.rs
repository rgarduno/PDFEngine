//! ISO 32000-1 §9.10.2 /ToUnicode CMap Parser and Bidirectional Character Mapper.
//!
//! Provides two-way translation between raw PDF character codes / CIDs and canonical Unicode strings.
//! Accurately decodes `beginbfchar ... endbfchar` and `beginbfrange ... endbfrange` blocks.

use std::collections::BTreeMap;
use crate::error::PdfResult;

/// Bidirectional mapping between PDF character codes and Unicode text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToUnicodeMap {
    /// Maps PDF character code / CID to Unicode string.
    pub char_to_unicode: BTreeMap<u32, String>,
    /// Reverse lookup: maps Unicode string to PDF character code / CID.
    pub unicode_to_char: BTreeMap<String, u32>,
}

impl ToUnicodeMap {
    /// Creates an empty mapping.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a mapping between a character code and a Unicode string.
    pub fn insert(&mut self, char_code: u32, unicode_str: String) {
        self.unicode_to_char.insert(unicode_str.clone(), char_code);
        self.char_to_unicode.insert(char_code, unicode_str);
    }

    /// Decodes a PDF character code into its corresponding Unicode string.
    pub fn decode_code(&self, code: u32) -> Option<&str> {
        self.char_to_unicode.get(&code).map(|s| s.as_str())
    }

    /// Reverse lookup: finds the PDF character code for a given Unicode character.
    pub fn encode_char(&self, unicode: &str) -> Option<u32> {
        self.unicode_to_char.get(unicode).copied()
    }

    /// Decodes a sequence of raw bytes into a Unicode string.
    ///
    /// # Arguments
    /// * `bytes` - The raw byte sequence from a `Tj` or `TJ` operand.
    /// * `bytes_per_char` - 1 for simple 8-bit fonts, 2 for CID/Type0 16-bit composite fonts.
    pub fn decode_bytes(&self, bytes: &[u8], bytes_per_char: usize) -> String {
        let mut out = String::new();
        if bytes_per_char == 2 {
            for chunk in bytes.chunks_exact(2) {
                let code = ((chunk[0] as u32) << 8) | (chunk[1] as u32);
                if let Some(s) = self.decode_code(code) {
                    out.push_str(s);
                } else {
                    out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                }
            }
        } else {
            for &b in bytes {
                let code = b as u32;
                if let Some(s) = self.decode_code(code) {
                    out.push_str(s);
                } else {
                    out.push(b as char);
                }
            }
        }
        out
    }

    /// Parses a `/ToUnicode` CMap stream from byte buffer.
    pub fn parse(cmap_data: &[u8]) -> PdfResult<Self> {
        let mut map = ToUnicodeMap::new();
        let content = String::from_utf8_lossy(cmap_data);
        let lines: Vec<&str> = content.lines().collect();

        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim();

            // 1. beginbfchar block: <srcCode> <dstUnicodeHex>
            if line.ends_with("beginbfchar") {
                let count: usize = line
                    .split_whitespace()
                    .next()
                    .and_then(|w| w.parse().ok())
                    .unwrap_or(0);

                i += 1;
                let mut processed = 0;
                while i < lines.len() && processed < count {
                    let entry = lines[i].trim();
                    if entry.starts_with("endbfchar") {
                        break;
                    }
                    if let Some((src_code, dst_str)) = Self::parse_bfchar_line(entry) {
                        map.insert(src_code, dst_str);
                        processed += 1;
                    }
                    i += 1;
                }
                continue;
            }

            // 2. beginbfrange block: <src1> <src2> <dstStart> OR <src1> <src2> [ <dst1> <dst2> ... ]
            if line.ends_with("beginbfrange") {
                let count: usize = line
                    .split_whitespace()
                    .next()
                    .and_then(|w| w.parse().ok())
                    .unwrap_or(0);

                i += 1;
                let mut processed = 0;
                while i < lines.len() && processed < count {
                    let entry = lines[i].trim();
                    if entry.starts_with("endbfrange") {
                        break;
                    }
                    Self::parse_bfrange_line(entry, &mut map);
                    processed += 1;
                    i += 1;
                }
                continue;
            }

            i += 1;
        }

        Ok(map)
    }

    /// Parses a single `<srcCode> <dstHex>` bfchar line.
    ///
    /// Whitespace between hex strings is optional, so `<0001><0041>` and
    /// `<0001> <0041>` are the same entry.
    fn parse_bfchar_line(line: &str) -> Option<(u32, String)> {
        let hexes = Self::hex_tokens(line);
        if hexes.len() < 2 {
            return None;
        }

        let src_code = u32::from_str_radix(&hexes[0], 16).ok()?;
        let dst_str = Self::hex_to_unicode_string(&hexes[1])?;

        Some((src_code, dst_str))
    }

    /// Parses `<src1> <src2> <dstStart>` or `<src1> <src2> [ ... ]` bfrange line.
    ///
    /// Writers such as Quartz emit `<21><21><0052>` with no space between the
    /// hex strings. Both forms are read by scanning `<...>` tokens.
    fn parse_bfrange_line(line: &str, map: &mut ToUnicodeMap) {
        let hexes = Self::hex_tokens(line);
        if hexes.len() < 3 {
            return;
        }

        let start_code = u32::from_str_radix(&hexes[0], 16).unwrap_or(0);
        let end_code = u32::from_str_radix(&hexes[1], 16).unwrap_or(0);
        if start_code > end_code {
            return;
        }

        if line.contains('[') {
            // Form 2: one destination string per source code.
            let mut code = start_code;
            for dst_hex in hexes.iter().skip(2) {
                if code > end_code {
                    break;
                }
                if let Some(unicode_str) = Self::hex_to_unicode_string(dst_hex) {
                    map.insert(code, unicode_str);
                }
                code += 1;
            }
        } else {
            // Form 1: the destination is one UTF-16BE string and the last unit increments.
            let mut dst_code = u32::from_str_radix(&hexes[2], 16).unwrap_or(0);
            for code in start_code..=end_code {
                if let Some(ch) = char::from_u32(dst_code) {
                    map.insert(code, ch.to_string());
                }
                dst_code += 1;
            }
        }
    }

    /// Hex digit runs inside `<...>`, ignoring whitespace inside each string.
    fn hex_tokens(line: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '<' {
                continue;
            }
            let mut hex = String::new();
            for inner in chars.by_ref() {
                if inner == '>' {
                    break;
                }
                if inner.is_ascii_hexdigit() {
                    hex.push(inner);
                }
            }
            if !hex.is_empty() {
                tokens.push(hex);
            }
        }
        tokens
    }

    /// Decodes a hexadecimal string into UTF-8 representation (UTF-16BE code units).
    fn hex_to_unicode_string(hex: &str) -> Option<String> {
        let mut u16_units = Vec::new();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .filter_map(|i| {
                if i + 2 <= hex.len() {
                    u8::from_str_radix(&hex[i..i + 2], 16).ok()
                } else {
                    None
                }
            })
            .collect();

        for chunk in bytes.chunks_exact(2) {
            u16_units.push(u16::from_be_bytes([chunk[0], chunk[1]]));
        }

        Some(String::from_utf16_lossy(&u16_units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tounicode_cmap_bfchar_and_bfrange() {
        let cmap = b"
/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
1 beginbfchar
<0001> <0041>
endbfchar
1 beginbfrange
<0002> <0004> <0042>
endbfrange
endcmap
end
";
        let map = ToUnicodeMap::parse(cmap).unwrap();

        // 0x0001 -> 'A' (0x0041)
        assert_eq!(map.decode_code(1), Some("A"));
        // Range: 0x0002 -> 'B', 0x0003 -> 'C', 0x0004 -> 'D'
        assert_eq!(map.decode_code(2), Some("B"));
        assert_eq!(map.decode_code(3), Some("C"));
        assert_eq!(map.decode_code(4), Some("D"));

        // Reverse lookup
        assert_eq!(map.encode_char("A"), Some(1));
        assert_eq!(map.encode_char("C"), Some(3));
    }

    #[test]
    fn parses_quartz_bfrange_without_spaces() {
        let cmap = b"
19 beginbfrange
<21><21><0052>
<22><22><0061>
<26><26><0020>
<2d><2e><0063>
<2f><2f><00F1>
endbfrange
1 beginbfchar
<0030><0045>
endbfchar
1 beginbfrange
<10><11>[<0041><0042>]
endbfrange
";
        let map = ToUnicodeMap::parse(cmap).unwrap();
        assert_eq!(map.decode_code(0x21), Some("R"));
        assert_eq!(map.decode_code(0x22), Some("a"));
        assert_eq!(map.decode_code(0x26), Some(" "));
        assert_eq!(map.decode_code(0x2d), Some("c"));
        assert_eq!(map.decode_code(0x2e), Some("d"));
        assert_eq!(map.decode_code(0x2f), Some("ñ"));
        assert_eq!(map.decode_code(0x30), Some("E"));
        assert_eq!(map.decode_code(0x10), Some("A"));
        assert_eq!(map.decode_code(0x11), Some("B"));
    }
}
