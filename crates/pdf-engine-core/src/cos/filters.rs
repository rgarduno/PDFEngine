//! Stream decompression filters according to ISO 32000-1 §7.4.
//!
//! Implements `FlateDecode` (zlib/deflate) with PNG/TIFF predictor algorithms,
//! `ASCIIHexDecode`, and `ASCII85Decode`.
//! All decoders enforce bounded memory allocation to prevent decompression bomb attacks.

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
pub use flate2::Compression;
use std::io::{Read, Write};

use crate::cos::object::PdfDictionary;
use crate::error::{PdfError, PdfResult};
use crate::security::SecurityLimits;

/// Encodes raw byte slice into `FlateDecode` (zlib) compressed data.
pub fn encode_flate(data: &[u8], level: Compression) -> std::io::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), level);
    encoder.write_all(data)?;
    encoder.finish()
}

/// Decodes stream bytes according to the specified filter name and optional parameters.
pub fn decode_stream(
    filter: &str,
    params: Option<&PdfDictionary>,
    data: &[u8],
    limits: &SecurityLimits,
) -> PdfResult<Vec<u8>> {
    match filter {
        "" | "Identity" => {
            limits.validate_decompression(data.len(), data.len())?;
            Ok(data.to_vec())
        }
        "FlateDecode" | "Fl" => decode_flate(data, params, limits),
        "ASCIIHexDecode" | "AHx" => decode_ascii_hex(data, limits),
        "ASCII85Decode" | "A85" => decode_ascii85(data, limits),
        _ => Err(PdfError::DecompressionError {
            filter: filter.to_string(),
            message: format!("Unsupported stream filter: '{}'", filter),
        }),
    }
}

/// Decodes `FlateDecode` compressed data with optional predictor post-processing (ISO 32000-1 §7.4.4).
pub fn decode_flate(
    data: &[u8],
    params: Option<&PdfDictionary>,
    limits: &SecurityLimits,
) -> PdfResult<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(data);
    let mut decompressed = Vec::new();

    // Use a bounded buffer reader to guard against memory exhaustion
    let mut buffer = [0u8; 8192];
    loop {
        match decoder.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                decompressed.extend_from_slice(&buffer[..n]);
                limits.validate_decompression(data.len(), decompressed.len())?;
            }
            Err(e) => {
                return Err(PdfError::DecompressionError {
                    filter: "FlateDecode".to_string(),
                    message: format!("Zlib decompression error: {}", e),
                });
            }
        }
    }

    // Process predictor if specified in DecodeParms
    if let Some(params) = params {
        let predictor = params
            .get("Predictor")
            .and_then(|o| o.as_i64())
            .unwrap_or(1);

        if predictor > 1 {
            let columns = predictor_dimension(params, "Columns", 1)?;
            let colors = predictor_dimension(params, "Colors", 1)?;
            let bits_per_component = predictor_dimension(params, "BitsPerComponent", 8)?;
            return apply_predictor(
                &decompressed,
                predictor,
                columns,
                colors,
                bits_per_component,
                limits,
            );
        }
    }

    Ok(decompressed)
}

/// Reads a predictor dimension. Missing keys use `default`. Zero and negative values are rejected
/// before they are cast: a zero `Columns` makes the row length 0, and `chunks_exact(0)` panics.
fn predictor_dimension(params: &PdfDictionary, key: &str, default: i64) -> PdfResult<usize> {
    let value = params.get(key).and_then(|o| o.as_i64()).unwrap_or(default);
    if value <= 0 {
        return Err(PdfError::DecompressionError {
            filter: "FlateDecode".to_string(),
            message: format!("Predictor /{} must be positive", key),
        });
    }
    usize::try_from(value).map_err(|_| {
        PdfError::SecurityLimitExceeded(format!(
            "Predictor /{} exceeds the decompression ceiling",
            key
        ))
    })
}

/// Applies PNG or TIFF predictor reconstruction (ISO 32000-1 §7.4.4.4).
fn apply_predictor(
    data: &[u8],
    predictor: i64,
    columns: usize,
    colors: usize,
    bits_per_component: usize,
    limits: &SecurityLimits,
) -> PdfResult<Vec<u8>> {
    let pixel_bits = colors
        .checked_mul(bits_per_component)
        .ok_or_else(predictor_overflow)?;
    let bytes_per_pixel = pixel_bits.saturating_add(7) / 8;
    let row_bits = columns
        .checked_mul(pixel_bits)
        .ok_or_else(predictor_overflow)?;
    let row_len = row_bits.saturating_add(7) / 8;
    if row_len == 0 || bytes_per_pixel == 0 {
        return Err(PdfError::DecompressionError {
            filter: "FlateDecode".to_string(),
            message: "Predictor row length is zero".to_string(),
        });
    }
    if row_len > limits.max_stream_decompressed_bytes {
        return Err(PdfError::SecurityLimitExceeded(format!(
            "Predictor row length ({} bytes) exceeds maximum allowable limit ({} bytes)",
            row_len, limits.max_stream_decompressed_bytes
        )));
    }

    // TIFF Predictor 2: Horizontal differencing
    if predictor == 2 {
        let mut out = data.to_vec();
        for row in out.chunks_exact_mut(row_len) {
            for i in bytes_per_pixel..row.len() {
                row[i] = row[i].wrapping_add(row[i - bytes_per_pixel]);
            }
        }
        return Ok(out);
    }

    // PNG Predictors 10 to 15
    if (10..=15).contains(&predictor) {
        let stride = row_len + 1; // 1 tag byte + row payload
        let mut out = Vec::with_capacity((data.len() / stride) * row_len);
        let mut prev_row = vec![0u8; row_len];

        for chunk in data.chunks_exact(stride) {
            let filter_type = chunk[0];
            let raw_row = &chunk[1..];
            let mut reconstructed_row = vec![0u8; row_len];

            for i in 0..row_len {
                let left = if i >= bytes_per_pixel {
                    reconstructed_row[i - bytes_per_pixel]
                } else {
                    0
                };
                let up = prev_row[i];
                let up_left = if i >= bytes_per_pixel {
                    prev_row[i - bytes_per_pixel]
                } else {
                    0
                };

                let val = match filter_type {
                    0 => raw_row[i],                                                     // PNG None
                    1 => raw_row[i].wrapping_add(left),                                  // PNG Sub
                    2 => raw_row[i].wrapping_add(up),                                    // PNG Up
                    3 => raw_row[i].wrapping_add(((left as u16 + up as u16) / 2) as u8), // PNG Average
                    4 => raw_row[i].wrapping_add(paeth_predictor(left, up, up_left)), // PNG Paeth
                    _ => {
                        return Err(PdfError::DecompressionError {
                            filter: "FlateDecode".to_string(),
                            message: format!("Unsupported PNG predictor tag: {}", filter_type),
                        });
                    }
                };
                reconstructed_row[i] = val;
            }

            out.extend_from_slice(&reconstructed_row);
            prev_row = reconstructed_row;
        }

        return Ok(out);
    }

    Ok(data.to_vec())
}

/// Reports a predictor dimension that overflowed while computing the row length.
fn predictor_overflow() -> PdfError {
    PdfError::SecurityLimitExceeded(
        "Predictor dimensions exceed the decompression ceiling".to_string(),
    )
}

/// Computes the Paeth filter prediction according to PNG specification.
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

/// Decodes `ASCIIHexDecode` byte stream (ISO 32000-1 §7.4.2).
pub fn decode_ascii_hex(data: &[u8], limits: &SecurityLimits) -> PdfResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut first_digit = None;

    for &b in data {
        if b == b'>' {
            break;
        }
        if b.is_ascii_whitespace() {
            continue;
        }

        let val = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => {
                return Err(PdfError::DecompressionError {
                    filter: "ASCIIHexDecode".to_string(),
                    message: format!("Invalid hex character: 0x{:02X}", b),
                });
            }
        };

        if let Some(first) = first_digit.take() {
            out.push((first << 4) | val);
            limits.validate_decompression(data.len(), out.len())?;
        } else {
            first_digit = Some(val);
        }
    }

    if let Some(first) = first_digit {
        out.push(first << 4);
    }

    Ok(out)
}

/// Decodes `ASCII85Decode` byte stream (ISO 32000-1 §7.4.3).
pub fn decode_ascii85(data: &[u8], limits: &SecurityLimits) -> PdfResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut tuple: u32 = 0;
    let mut count = 0;

    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        i += 1;

        if b == b'~' && i < data.len() && data[i] == b'>' {
            break;
        }
        if b.is_ascii_whitespace() {
            continue;
        }

        if b == b'z' && count == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            limits.validate_decompression(data.len(), out.len())?;
            continue;
        }

        if !(b'!'..=b'u').contains(&b) {
            return Err(PdfError::DecompressionError {
                filter: "ASCII85Decode".to_string(),
                message: format!("Invalid ASCII85 character: 0x{:02X}", b),
            });
        }

        tuple = tuple * 85 + (b - b'!') as u32;
        count += 1;

        if count == 5 {
            out.extend_from_slice(&tuple.to_be_bytes());
            limits.validate_decompression(data.len(), out.len())?;
            tuple = 0;
            count = 0;
        }
    }

    if count > 1 {
        for _ in 0..(5 - count) {
            tuple = tuple * 85 + 84;
        }
        let bytes = tuple.to_be_bytes();
        out.extend_from_slice(&bytes[..count - 1]);
        limits.validate_decompression(data.len(), out.len())?;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn test_flate_roundtrip() {
        let limits = SecurityLimits::default();
        let original_data = b"Hello, PDF ISO 32000 Flate Stream Compression!";

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original_data).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decode_flate(&compressed, None, &limits).unwrap();
        assert_eq!(decompressed, original_data);
    }

    #[test]
    fn test_ascii_hex_decode() {
        let limits = SecurityLimits::default();
        let hex_data = b"48656c6c6f20576f726c64>";
        let decoded = decode_ascii_hex(hex_data, &limits).unwrap();
        assert_eq!(decoded, b"Hello World");
    }

    #[test]
    fn predictor_columns_of_zero_is_rejected() {
        let limits = SecurityLimits::default();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"abcdefghijklmnop").unwrap();
        let compressed = encoder.finish().unwrap();

        let mut params = PdfDictionary::new();
        params.insert("Predictor", 2i64);
        params.insert("Columns", 0i64);
        let error = decode_flate(&compressed, Some(&params), &limits).unwrap_err();
        assert!(error.to_string().contains("Columns"));
    }

    #[test]
    fn predictor_row_above_the_ceiling_is_rejected() {
        let mut limits = SecurityLimits::default();
        limits.max_stream_decompressed_bytes = 64;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"abcdefghijklmnop").unwrap();
        let compressed = encoder.finish().unwrap();

        let mut params = PdfDictionary::new();
        params.insert("Predictor", 2i64);
        params.insert("Columns", 10_000i64);
        let error = decode_flate(&compressed, Some(&params), &limits).unwrap_err();
        assert!(error.to_string().contains("Predictor row length"));
    }
}
