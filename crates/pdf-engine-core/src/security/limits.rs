//! Security policies and resource limits for robust PDF ingestion.
//!
//! Enforces bounded memory allocation, decompression quotas, and recursion guards
//! to prevent Denial of Service (DoS) attacks and resource exhaustion in cloud
//! and serverless environments.

use crate::error::{PdfError, PdfResult};

/// Configuration parameters governing security enforcement during PDF ingestion.
#[derive(Debug, Clone)]
pub struct SecurityLimits {
    /// Maximum allowed decompressed byte size for any single stream (default: 256 MiB).
    pub max_stream_decompressed_bytes: usize,

    /// Maximum allowed expansion ratio between compressed and decompressed stream bytes (default: 100x).
    pub max_decompression_ratio: usize,

    /// Maximum recursion depth allowed during object resolution and dictionary traversal (default: 64).
    pub max_recursion_depth: usize,

    /// Maximum number of total objects permitted in a single document (default: 500,000).
    pub max_object_count: usize,
}

impl Default for SecurityLimits {
    fn default() -> Self {
        Self {
            max_stream_decompressed_bytes: 256 * 1024 * 1024, // 256 MiB
            max_decompression_ratio: 100,                     // 100:1 ratio
            max_recursion_depth: 64,                          // 64 nested levels
            max_object_count: 500_000,
        }
    }
}

/// Compressed inputs at or below this size may exceed the ratio until the
/// output passes [`SMALL_STREAM_OUTPUT_FLOOR`].
const SMALL_STREAM_BYTES: usize = 1024;

/// Output size at which the ratio applies even to a small compressed input.
const SMALL_STREAM_OUTPUT_FLOOR: usize = 1024 * 1024;

impl SecurityLimits {
    /// Validates whether a decompressed stream size remains within allowable security bounds.
    ///
    /// The absolute ceiling is checked first. The expansion ratio then applies
    /// to every compressed input larger than 1 KiB. A smaller input may exceed
    /// the ratio until its output passes 1 MiB, which keeps a short legitimate
    /// header from being rejected while a 1 KiB input cannot grow toward the
    /// ceiling. An empty compressed input that still yields output is rejected.
    ///
    /// # Arguments
    /// * `compressed_size` - Size in bytes of the compressed stream source.
    /// * `decompressed_size` - Size in bytes of the expanded stream output.
    pub fn validate_decompression(
        &self,
        compressed_size: usize,
        decompressed_size: usize,
    ) -> PdfResult<()> {
        if decompressed_size > self.max_stream_decompressed_bytes {
            return Err(PdfError::SecurityLimitExceeded(format!(
                "Decompressed stream size ({} bytes) exceeds maximum allowable limit ({} bytes)",
                decompressed_size, self.max_stream_decompressed_bytes
            )));
        }

        if compressed_size == 0 {
            if decompressed_size > 0 {
                return Err(PdfError::SecurityLimitExceeded(format!(
                    "Decompression expansion ratio is undefined for an empty compressed stream of {decompressed_size} bytes"
                )));
            }
            return Ok(());
        }

        let ratio = decompressed_size / compressed_size;
        let small_input = compressed_size <= SMALL_STREAM_BYTES;
        let under_floor = decompressed_size <= SMALL_STREAM_OUTPUT_FLOOR;
        if ratio > self.max_decompression_ratio && !(small_input && under_floor) {
            return Err(PdfError::SecurityLimitExceeded(format!(
                "Decompression expansion ratio ({}x) exceeds maximum allowable safety ratio ({}x)",
                ratio, self.max_decompression_ratio
            )));
        }

        Ok(())
    }

    /// Rejects a document whose object count is above `max_object_count`.
    pub fn validate_object_count(&self, count: usize) -> PdfResult<()> {
        if count > self.max_object_count {
            return Err(PdfError::SecurityLimitExceeded(format!(
                "Object count ({}) exceeds maximum allowable limit ({})",
                count, self.max_object_count
            )));
        }
        Ok(())
    }

    /// Validates whether the current traversal depth does not exceed maximum recursion threshold.
    pub fn validate_depth(&self, depth: usize, id: u32, gen: u16) -> PdfResult<()> {
        if depth > self.max_recursion_depth {
            return Err(PdfError::RecursionLimitExceeded {
                id,
                gen,
                max_depth: self.max_recursion_depth,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_stream_may_exceed_the_ratio_under_one_mebibyte() {
        let limits = SecurityLimits::default();
        limits
            .validate_decompression(100, 50 * 1024)
            .expect("50 KiB from 100 bytes");
        limits
            .validate_decompression(SMALL_STREAM_BYTES, SMALL_STREAM_OUTPUT_FLOOR)
            .expect("the floor itself is still accepted");
    }

    #[test]
    fn small_stream_above_the_floor_is_rejected() {
        let limits = SecurityLimits::default();
        let err = limits
            .validate_decompression(100, SMALL_STREAM_OUTPUT_FLOOR + 1)
            .expect_err("a small input cannot expand past the floor");
        match err {
            PdfError::SecurityLimitExceeded(msg) => assert!(msg.contains("expansion ratio")),
            other => panic!("expected a ratio limit, got {other:?}"),
        }
    }

    #[test]
    fn empty_compressed_stream_with_output_is_rejected() {
        let limits = SecurityLimits::default();
        limits.validate_decompression(0, 0).expect("empty to empty");
        let err = limits
            .validate_decompression(0, 1)
            .expect_err("empty input cannot yield output");
        assert!(matches!(err, PdfError::SecurityLimitExceeded(_)));
    }

    #[test]
    fn input_above_one_kibibyte_keeps_the_ratio() {
        let limits = SecurityLimits::default();
        let compressed = SMALL_STREAM_BYTES + 1;
        limits
            .validate_decompression(compressed, compressed * limits.max_decompression_ratio)
            .expect("the ratio boundary is accepted");
        let err = limits
            .validate_decompression(
                compressed,
                compressed * (limits.max_decompression_ratio + 1),
            )
            .expect_err("one step past the ratio is rejected");
        assert!(matches!(err, PdfError::SecurityLimitExceeded(_)));
    }
}
