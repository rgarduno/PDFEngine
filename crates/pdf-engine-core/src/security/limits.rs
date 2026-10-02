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

impl SecurityLimits {
    /// Validates whether a decompressed stream size remains within allowable security bounds.
    ///
    /// # Arguments
    /// * `compressed_size` - Size in bytes of the compressed stream source.
    /// * `decompressed_size` - Size in bytes of the expanded stream output.
    pub fn validate_decompression(&self, compressed_size: usize, decompressed_size: usize) -> PdfResult<()> {
        if decompressed_size > self.max_stream_decompressed_bytes {
            return Err(PdfError::SecurityLimitExceeded(format!(
                "Decompressed stream size ({} bytes) exceeds maximum allowable limit ({} bytes)",
                decompressed_size, self.max_stream_decompressed_bytes
            )));
        }

        // Apply ratio check only if compressed size is non-trivial (> 1 KiB) to avoid false positives on tiny headers
        if compressed_size > 1024 {
            let ratio = decompressed_size / compressed_size;
            if ratio > self.max_decompression_ratio {
                return Err(PdfError::SecurityLimitExceeded(format!(
                    "Decompression expansion ratio ({}x) exceeds maximum allowable safety ratio ({}x)",
                    ratio, self.max_decompression_ratio
                )));
            }
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
