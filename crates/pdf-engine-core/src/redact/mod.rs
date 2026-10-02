//! PDF Redaction and Content Sanitization engine according to ISO 32000-1 §14.11.
//!
//! Provides true irreversible content surgery (purging text and glyphs from streams),
//! vector blackout box synthesis, PII pattern scanners (email, phone, SSN, cards, RFC, CURP),
//! annotation pruning, and metadata scrubbing.

pub mod patterns;
pub mod sanitizer;
pub mod surgery;
pub mod types;

#[cfg(test)]
mod tests;

pub use patterns::{
    find_credit_cards, find_curp, find_emails, find_matches, find_phones, find_rfc, find_ssn,
    find_substring,
};
pub use sanitizer::{
    prune_page_annotations, redact_document_pattern, redact_document_rectangles, redact_page,
    scrub_document_metadata,
};
pub use surgery::{apply_redaction_to_ast, find_pattern_boxes_on_page};
pub use types::{RedactionConfig, RedactionPattern, RedactionRect, RedactionSummary};
