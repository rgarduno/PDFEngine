//! PNG (W3C RFC 2083) parser and encoder.
//!
//! Handles chunk processing, scanline reconstruction (Sub, Up, Average, Paeth filters),
//! alpha transparency separation for `/SMask`, and lossless PNG synthesis.

use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

use crate::cos::filters::decode_flate;
use crate::error::{PdfError, PdfResult};
use crate::security::SecurityLimits;

/// PNG header metadata from the `IHDR` chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct PngHeader {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
}

/// Standard 8-byte PNG file signature.
pub const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Parses the `IHDR` header of a PNG file.
pub fn parse_png_header(data: &[u8]) -> PdfResult<PngHeader> {
    if data.len() < 33 || &data[0..8] != PNG_SIGNATURE {
        return Err(PdfError::ParseError {
            offset: 0,
            message: "Invalid PNG signature".to_string(),
        });
    }

    let ihdr_len = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
    if ihdr_len != 13 || &data[12..16] != b"IHDR" {
        return Err(PdfError::ParseError {
            offset: 8,
            message: "First PNG chunk is not a valid IHDR chunk".to_string(),
        });
    }

    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    let bit_depth = data[24];
    let color_type = data[25];

    Ok(PngHeader {
        width,
        height,
        bit_depth,
        color_type,
    })
}

/// Parses and unfilters pixel data from a PNG file.
///
/// IDAT inflation uses the same ceiling as other Flate streams. The declared
/// width and height are rejected when the reconstructed image would pass that ceiling.
/// Returns `(width, height, rgb_or_gray_samples, optional_alpha_samples)`.
pub fn parse_png_pixels(
    data: &[u8],
    limits: &SecurityLimits,
) -> PdfResult<(u32, u32, Vec<u8>, Option<Vec<u8>>)> {
    let header = parse_png_header(data)?;

    if header.bit_depth != 8 {
        return Err(PdfError::ParseError {
            offset: 24,
            message: format!(
                "Unsupported PNG bit depth: {} (expected 8-bit)",
                header.bit_depth
            ),
        });
    }

    let bpp: usize = match header.color_type {
        0 => 1, // Grayscale
        2 => 3, // Truecolor RGB
        4 => 2, // Grayscale + Alpha
        6 => 4, // Truecolor RGBA
        other => {
            return Err(PdfError::ParseError {
                offset: 25,
                message: format!("Unsupported PNG color type: {}", other),
            });
        }
    };

    // 1. Gather all concatenated IDAT payload data
    let mut idat_data = Vec::new();
    let mut offset = 8;
    while offset + 8 <= data.len() {
        let length = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let chunk_type = &data[offset + 4..offset + 8];
        let chunk_start = offset + 8;
        let chunk_end = chunk_start + length;

        if chunk_end + 4 > data.len() {
            return Err(PdfError::ParseError {
                offset,
                message: "PNG chunk bounds exceed buffer size".to_string(),
            });
        }

        if chunk_type == b"IDAT" {
            idat_data.extend_from_slice(&data[chunk_start..chunk_end]);
        } else if chunk_type == b"IEND" {
            break;
        }

        offset = chunk_end + 4; // Skip CRC
    }

    // 2. Decompress zlib stream under the same ceiling as FlateDecode.
    let decompressed = decode_flate(&idat_data, None, limits).map_err(|error| match error {
        PdfError::DecompressionError { message, .. } => PdfError::DecompressionError {
            filter: "FlateDecode".to_string(),
            message: format!("Failed to decompress PNG IDAT data: {}", message),
        },
        other => other,
    })?;

    let width = header.width as usize;
    let height = header.height as usize;
    let row_len = width.checked_mul(bpp).ok_or_else(|| {
        PdfError::SecurityLimitExceeded(
            "PNG row width exceeds the decompression ceiling".to_string(),
        )
    })?;
    let scanline = row_len.checked_add(1).ok_or_else(|| {
        PdfError::SecurityLimitExceeded(
            "PNG row width exceeds the decompression ceiling".to_string(),
        )
    })?;
    let expected_len = height.checked_mul(scanline).ok_or_else(|| {
        PdfError::SecurityLimitExceeded(
            "PNG image size exceeds the decompression ceiling".to_string(),
        )
    })?;
    limits.validate_decompression(idat_data.len(), expected_len)?;

    if decompressed.len() < expected_len {
        return Err(PdfError::ParseError {
            offset: 0,
            message: format!(
                "PNG decompressed size {} smaller than required {}",
                decompressed.len(),
                expected_len
            ),
        });
    }

    // 3. Unfilter scanlines according to W3C PNG filter types
    let mut reconstructed = Vec::with_capacity(width * height * bpp);
    let mut prev_row = vec![0u8; row_len];
    let mut src_offset = 0;

    for _y in 0..height {
        let filter_type = decompressed[src_offset];
        src_offset += 1;
        let raw_row = &decompressed[src_offset..src_offset + row_len];
        src_offset += row_len;

        let mut current_row = vec![0u8; row_len];

        match filter_type {
            0 => {
                // None
                current_row.copy_from_slice(raw_row);
            }
            1 => {
                // Sub: raw + Recon(x - bpp)
                for x in 0..row_len {
                    let a = if x >= bpp { current_row[x - bpp] } else { 0 };
                    current_row[x] = raw_row[x].wrapping_add(a);
                }
            }
            2 => {
                // Up: raw + Prior(x)
                for x in 0..row_len {
                    let b = prev_row[x];
                    current_row[x] = raw_row[x].wrapping_add(b);
                }
            }
            3 => {
                // Average: raw + floor((Recon(x - bpp) + Prior(x)) / 2)
                for x in 0..row_len {
                    let a = if x >= bpp {
                        current_row[x - bpp] as u16
                    } else {
                        0
                    };
                    let b = prev_row[x] as u16;
                    let avg = ((a + b) / 2) as u8;
                    current_row[x] = raw_row[x].wrapping_add(avg);
                }
            }
            4 => {
                // Paeth
                for x in 0..row_len {
                    let a = if x >= bpp { current_row[x - bpp] } else { 0 };
                    let b = prev_row[x];
                    let c = if x >= bpp { prev_row[x - bpp] } else { 0 };
                    let p = paeth_predictor(a, b, c);
                    current_row[x] = raw_row[x].wrapping_add(p);
                }
            }
            other => {
                return Err(PdfError::ParseError {
                    offset: src_offset - row_len - 1,
                    message: format!("Unknown PNG scanline filter: {}", other),
                });
            }
        }

        reconstructed.extend_from_slice(&current_row);
        prev_row = current_row;
    }

    // 4. Separate color channels and optional Alpha transparency channel
    if header.color_type == 6 {
        // RGBA -> RGB (3 channels) + Alpha (1 channel for /SMask)
        let num_pixels = width * height;
        let mut rgb = Vec::with_capacity(num_pixels * 3);
        let mut alpha = Vec::with_capacity(num_pixels);

        for chunk in reconstructed.chunks_exact(4) {
            rgb.push(chunk[0]);
            rgb.push(chunk[1]);
            rgb.push(chunk[2]);
            alpha.push(chunk[3]);
        }
        Ok((header.width, header.height, rgb, Some(alpha)))
    } else if header.color_type == 4 {
        // Gray + Alpha
        let num_pixels = width * height;
        let mut gray = Vec::with_capacity(num_pixels);
        let mut alpha = Vec::with_capacity(num_pixels);

        for chunk in reconstructed.chunks_exact(2) {
            gray.push(chunk[0]);
            alpha.push(chunk[1]);
        }
        Ok((header.width, header.height, gray, Some(alpha)))
    } else {
        // Standard RGB or Grayscale without alpha
        Ok((header.width, header.height, reconstructed, None))
    }
}

/// Standard Paeth predictor calculation for PNG filter type 4.
#[inline]
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let pa = (p - a as i32).abs();
    let pb = (p - b as i32).abs();
    let pc = (p - c as i32).abs();

    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Encodes raw pixel samples (RGB or Grayscale) into a standard compliant PNG byte stream.
pub fn encode_png(width: u32, height: u32, pixel_data: &[u8], is_rgb: bool) -> PdfResult<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR chunk
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // Bit depth
    ihdr.push(if is_rgb { 2 } else { 0 }); // Color type: 2=RGB, 0=Grayscale
    ihdr.push(0); // Deflate
    ihdr.push(0); // Filter
    ihdr.push(0); // No interlace
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Prepare scanlines: Prepend 0x00 (Filter: None) to each row
    let bytes_per_pixel = if is_rgb { 3 } else { 1 };
    let row_len = width as usize * bytes_per_pixel;
    let mut filtered = Vec::with_capacity((row_len + 1) * height as usize);

    for row in pixel_data.chunks(row_len) {
        filtered.push(0); // Filter type 0: None
        filtered.extend_from_slice(row);
    }

    // Compress with zlib Deflate
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&filtered)
        .map_err(|e| PdfError::DecompressionError {
            filter: "FlateDecode".to_string(),
            message: format!("PNG compression error: {}", e),
        })?;
    let idat = encoder.finish().map_err(|e| PdfError::DecompressionError {
        filter: "FlateDecode".to_string(),
        message: format!("PNG compression finish error: {}", e),
    })?;

    write_chunk(&mut out, b"IDAT", &idat);
    write_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

/// Writes a typed PNG chunk with 32-bit big-endian length, chunk identifier, payload, and CRC32.
fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);

    // Compute CRC32 over chunk_type + data
    let mut crc = 0xFFFF_FFFFu32;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc = !crc;
    out.extend_from_slice(&crc.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_png_encode_and_parse_roundtrip() {
        let width = 4;
        let height = 2;
        // 4x2 RGB pixels (8 pixels * 3 = 24 bytes)
        let original_pixels = vec![
            255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 128, 128, 128, 64, 64, 64, 32, 32, 32, 0,
            0, 0,
        ];

        let png_bytes = encode_png(width, height, &original_pixels, true).expect("Encode PNG");
        assert!(png_bytes.starts_with(&PNG_SIGNATURE));

        let (parsed_w, parsed_h, parsed_pixels, alpha) =
            parse_png_pixels(&png_bytes, &SecurityLimits::default()).expect("Parse PNG");

        assert_eq!(parsed_w, width);
        assert_eq!(parsed_h, height);
        assert_eq!(parsed_pixels, original_pixels);
        assert!(alpha.is_none());
    }

    #[test]
    fn png_idat_stops_at_the_decompression_ceiling() {
        let width = 4u32;
        let height = 2u32;
        let original_pixels = vec![0u8; (width * height * 3) as usize];
        let png_bytes = encode_png(width, height, &original_pixels, true).expect("Encode PNG");

        let mut limits = SecurityLimits::default();
        limits.max_stream_decompressed_bytes = 8;
        let error = parse_png_pixels(&png_bytes, &limits).unwrap_err();
        assert!(error
            .to_string()
            .contains("exceeds maximum allowable limit"));
    }
}
