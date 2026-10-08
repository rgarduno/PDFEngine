//! JPEG (JFIF / ISO 10918-1) parsing according to ISO 32000-1 §7.4.8 (`/DCTDecode`).
//!
//! Provides zero-copy header inspection for baseline and progressive JPEG bitstreams
//! to extract image dimensions, color components, and sample precision.

use crate::error::{PdfError, PdfResult};

/// Extracted metadata from a JPEG Start of Frame (SOF) marker.
#[derive(Debug, Clone, PartialEq)]
pub struct JpegHeader {
    /// Horizontal pixel dimension.
    pub width: u32,
    /// Vertical pixel dimension.
    pub height: u32,
    /// Number of color components (1 = Grayscale, 3 = RGB / YCbCr, 4 = CMYK).
    pub components: u8,
    /// Sample bit precision (typically 8 bits per sample).
    pub precision: u8,
}

/// Parses a JPEG byte sequence to extract its SOF metadata.
pub fn parse_jpeg(data: &[u8]) -> PdfResult<JpegHeader> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(PdfError::ParseError {
            offset: 0,
            message: "Invalid JPEG: missing Start of Image (SOI 0xFFD8) signature".to_string(),
        });
    }

    let mut offset = 2;
    while offset < data.len() {
        if data[offset] != 0xFF {
            offset += 1;
            continue;
        }

        // Advance through any fill bytes (0xFF)
        while offset < data.len() && data[offset] == 0xFF {
            offset += 1;
        }
        if offset >= data.len() {
            break;
        }

        let marker = data[offset];
        offset += 1;

        // Restart markers (RST0..RST7), SOI (0xD8), EOI (0xD9) have no payload length
        if (0xD0..=0xD7).contains(&marker) || marker == 0xD8 || marker == 0xD9 {
            continue;
        }

        if offset + 2 > data.len() {
            break;
        }
        let length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        if length < 2 || offset + length > data.len() {
            return Err(PdfError::ParseError {
                offset,
                message: "Corrupted JPEG segment length exceeds buffer bounds".to_string(),
            });
        }

        // Start of Frame markers:
        // SOF0 (0xC0: Baseline), SOF1 (0xC1: Extended Sequential), SOF2 (0xC2: Progressive)
        if (0xC0..=0xC3).contains(&marker)
            || (0xC5..=0xC7).contains(&marker)
            || (0xC9..=0xCB).contains(&marker)
            || (0xCD..=0xCF).contains(&marker)
        {
            if length < 8 {
                return Err(PdfError::ParseError {
                    offset,
                    message: "SOF segment too short to contain dimensions".to_string(),
                });
            }
            let precision = data[offset + 2];
            let height = u16::from_be_bytes([data[offset + 3], data[offset + 4]]) as u32;
            let width = u16::from_be_bytes([data[offset + 5], data[offset + 6]]) as u32;
            let components = data[offset + 7];

            return Ok(JpegHeader {
                width,
                height,
                components,
                precision,
            });
        }

        offset += length;
    }

    Err(PdfError::ParseError {
        offset,
        message: "No Start of Frame (SOF) marker found in JPEG stream".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_jpeg_header() {
        // Construct minimal valid JPEG with SOI, APP0, SOF0, EOI
        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(&[0xFF, 0xD8]); // SOI

        // SOF0 marker: length=11, precision=8, height=480, width=640, components=3
        jpeg.extend_from_slice(&[0xFF, 0xC0]);
        jpeg.extend_from_slice(&8u16.to_be_bytes()); // length = 8
        jpeg.push(8); // 8 bits precision
        jpeg.extend_from_slice(&480u16.to_be_bytes());
        jpeg.extend_from_slice(&640u16.to_be_bytes());
        jpeg.push(3); // 3 components (RGB)

        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI

        let header = parse_jpeg(&jpeg).expect("Must parse valid JPEG header");
        assert_eq!(header.width, 640);
        assert_eq!(header.height, 480);
        assert_eq!(header.components, 3);
        assert_eq!(header.precision, 8);
    }
}
