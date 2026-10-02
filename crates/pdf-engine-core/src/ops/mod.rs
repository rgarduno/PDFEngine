//! Document assembly and page-level manipulation operations according to ISO 32000-1.
//!
//! Provides zero-loss document merging, page splitting/extraction, page rotation,
//! reordering, and deletion with topological reference graph preservation.

pub mod cloner;
pub mod merge;
pub mod reorder;
pub mod rotation;
pub mod split;

pub use cloner::ObjectCloner;
pub use merge::{merge_documents, merge_pdf_bytes};
pub use reorder::{delete_pages, reorder_pages};
pub use rotation::{
    get_page_rotation, normalize_rotation, rotate_all_pages, rotate_page, set_page_rotation,
};
pub use split::{extract_pages, split_by_ranges, split_document};

#[cfg(test)]
mod tests;
