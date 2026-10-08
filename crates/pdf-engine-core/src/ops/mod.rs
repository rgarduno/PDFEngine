//! Document assembly and page-level manipulation operations according to ISO 32000-1.
//!
//! Provides zero-loss document merging, page splitting/extraction, page rotation,
//! reordering, and deletion with topological reference graph preservation.

pub mod cloner;
pub mod diff;
pub mod merge;
pub mod metadata;
pub mod optimize;
pub mod reorder;
pub mod rotation;
pub mod split;

pub use cloner::ObjectCloner;
pub use diff::{
    compare_documents, compute_word_diffs, DiffKind, DiffOptions, DiffReport, DiffSummary,
    ImageDiffItem, MetadataDiffItem, PageDiff, PageDimensions, TextDiffItem, WordDiff,
};
pub use merge::{merge_documents, merge_pdf_bytes};
pub use metadata::{
    current_timestamps, extract_metadata, format_utc_timestamps, iso_to_pdf_date, pdf_date_to_iso,
    update_metadata, DocumentMetadata,
};
pub use optimize::{
    collect_garbage, deduplicate_streams, optimize_document, recompress_streams,
    save_optimized_to_vec, OptimizationOptions, OptimizationStats,
};
pub use reorder::{delete_pages, reorder_pages};
pub use rotation::{
    get_page_rotation, normalize_rotation, rotate_all_pages, rotate_page, set_page_rotation,
};
pub use split::{extract_pages, split_by_ranges, split_document};

#[cfg(test)]
mod tests;
