//! Glyph excision, PII scanning, and optional document-metadata scrubbing.
//!
//! A redaction removes intersecting glyphs from the page content stream, draws an
//! opaque blackout, and can drop intersecting annotations. It also removes page
//! `/Metadata` and marked-content `/ActualText`, `/Alt`, and `/E` on that page.
//! An embedded file is removed when its name, filename, description, or decoded
//! bytes repeat a removed string of at least 4 characters. A form field loses
//! `/V` and `/AP` when either one repeats such a string. The widget stays. The
//! structure tree stays. Document `/Info` and catalog XMP are removed only
//! when `scrub_metadata` is set. This is not an ISO 32000-1 legal redaction.

mod embedded;
mod fields;
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
