//! TrueType and OpenType (SFNT) binary table parser according to ISO/IEC 14496-22.
//!
//! Extracts glyph metrics, bounding boxes, and Unicode-to-GlyphID character mappings
//! from embedded `head`, `hhea`, `hmtx`, and `cmap` tables.

use std::collections::BTreeMap;
use crate::error::{PdfError, PdfResult};

/// Represents an offset and length slice of an SFNT table within a font binary.
#[derive(Debug, Clone, Copy)]
pub struct TableRecord {
    pub offset: usize,
    pub length: usize,
}

/// Parsed OpenType / TrueType font descriptor.
#[derive(Debug, Clone)]
pub struct SfntFont {
    /// Mapping of 4-byte table tags (e.g. `head`, `hmtx`) to their binary records.
    pub tables: BTreeMap<[u8; 4], TableRecord>,
    /// Number of design units per em square (typically 1000 or 2048 from `head` table).
    pub units_per_em: u16,
    /// Number of horizontal metrics entries from `hhea` table.
    pub num_h_metrics: u16,
}

impl SfntFont {
    /// Parses an SFNT font header and table directory.
    pub fn parse(font_data: &[u8]) -> PdfResult<Self> {
        if font_data.len() < 12 {
            return Err(PdfError::FontError {
                font_name: "SFNT".to_string(),
                message: "Buffer too small for SFNT header".to_string(),
            });
        }

        let num_tables = u16::from_be_bytes([font_data[4], font_data[5]]) as usize;
        let mut tables = BTreeMap::new();

        let mut offset = 12;
        for _ in 0..num_tables {
            if offset + 16 > font_data.len() {
                break;
            }

            let tag: [u8; 4] = [
                font_data[offset],
                font_data[offset + 1],
                font_data[offset + 2],
                font_data[offset + 3],
            ];
            let tbl_offset = u32::from_be_bytes([
                font_data[offset + 8],
                font_data[offset + 9],
                font_data[offset + 10],
                font_data[offset + 11],
            ]) as usize;
            let tbl_len = u32::from_be_bytes([
                font_data[offset + 12],
                font_data[offset + 13],
                font_data[offset + 14],
                font_data[offset + 15],
            ]) as usize;

            tables.insert(tag, TableRecord { offset: tbl_offset, length: tbl_len });
            offset += 16;
        }

        // Parse unitsPerEm from `head` table
        let mut units_per_em = 1000;
        if let Some(head) = tables.get(b"head") {
            if head.offset + 54 <= font_data.len() {
                // unitsPerEm is at offset 18 within head table
                units_per_em = u16::from_be_bytes([
                    font_data[head.offset + 18],
                    font_data[head.offset + 19],
                ]);
            }
        }

        // Parse numOfLongHorMetrics from `hhea` table
        let mut num_h_metrics = 0;
        if let Some(hhea) = tables.get(b"hhea") {
            if hhea.offset + 36 <= font_data.len() {
                // numberOfHMetrics is at offset 34 within hhea table
                num_h_metrics = u16::from_be_bytes([
                    font_data[hhea.offset + 34],
                    font_data[hhea.offset + 35],
                ]);
            }
        }

        Ok(Self {
            tables,
            units_per_em,
            num_h_metrics,
        })
    }

    /// Retrieves the horizontal advance width for a specific GlyphID in font design units.
    pub fn get_glyph_advance(&self, glyph_id: u16, font_data: &[u8]) -> Option<u16> {
        let hmtx = self.tables.get(b"hmtx")?;
        let gid = glyph_id as usize;

        if gid < self.num_h_metrics as usize {
            // First num_h_metrics entries have 4 bytes each: [advanceWidth (2B), lsb (2B)]
            let entry_offset = hmtx.offset + gid * 4;
            if entry_offset + 2 <= font_data.len() {
                return Some(u16::from_be_bytes([
                    font_data[entry_offset],
                    font_data[entry_offset + 1],
                ]));
            }
        } else if self.num_h_metrics > 0 {
            // Remaining glyphs share the advance width of the last long horizontal metric entry
            let last_idx = (self.num_h_metrics - 1) as usize;
            let entry_offset = hmtx.offset + last_idx * 4;
            if entry_offset + 2 <= font_data.len() {
                return Some(u16::from_be_bytes([
                    font_data[entry_offset],
                    font_data[entry_offset + 1],
                ]));
            }
        }

        None
    }
}
