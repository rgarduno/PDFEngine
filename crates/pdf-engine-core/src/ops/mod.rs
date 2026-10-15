//! Document assembly and page-level manipulation operations according to ISO 32000-1.
//!
//! Provides zero-loss document merging, page splitting/extraction, page rotation,
//! reordering, and deletion with topological reference graph preservation.

pub mod attachments;
pub mod cloner;
pub mod compose;
pub mod diff;
pub mod export_docx;
pub mod flatten;
pub mod layers;
pub mod merge;
pub mod metadata;
pub mod optimize;
pub mod outlines;
pub mod raster;
pub mod reorder;
pub mod rotation;
pub mod replace;
pub mod search;
pub mod split;

pub use attachments::{
    add_attachment, delete_attachment, extract_attachment, list_attachments, EmbeddedAttachment,
    EmbeddedAttachmentInfo,
};
pub use cloner::ObjectCloner;
pub use compose::{
    duplicate_page, insert_blank_page, insert_pages, page_boxes, parse_page_boundary,
    reverse_pages, set_page_boundary, PageBoundary, PageBoxes,
};
pub use diff::{
    compare_documents, compute_word_diffs, DiffKind, DiffOptions, DiffReport, DiffSummary,
    ImageDiffItem, MetadataDiffItem, PageDiff, PageDimensions, TextDiffItem, WordDiff,
};
pub use export_docx::{export_document_docx, export_document_markdown};
pub use flatten::{flatten_document_permanently, HardFlattenReport};
pub use layers::{
    add_layer, delete_layer, extract_layers, register_page_layer_property,
    resolve_page_hidden_ocg_props, set_layer_visibility, OptionalContentLayer,
};
pub use merge::{append_document, merge_documents, merge_pdf_bytes};
pub use metadata::{
    current_timestamps, extract_metadata, format_utc_timestamps, iso_to_pdf_date, pdf_date_to_iso,
    update_metadata, DocumentMetadata,
};
pub use optimize::{
    collect_garbage, deduplicate_streams, optimize_document, recompress_streams,
    save_optimized_to_vec, OptimizationOptions, OptimizationStats,
};
pub use outlines::{add_outline_item, clear_outlines, extract_outlines, set_outlines, OutlineItem};
pub use raster::{
    render_page, PageRaster, RasterFormat, DEFAULT_RASTER_DPI, MAX_RASTER_DPI, MAX_RASTER_PIXELS,
    MAX_RASTER_SIDE, MIN_RASTER_DPI,
};
pub use reorder::{delete_pages, reorder_pages};
pub use rotation::{
    get_page_rotation, normalize_rotation, rotate_all_pages, rotate_page, set_page_rotation,
};
pub use replace::{replace_document_text, TextReplaceReport};
pub use search::{
    search_document, TextSearchHit, TextSearchReport, MAX_QUERY_SCALARS, MAX_SEARCH_HITS,
};
pub use split::{extract_pages, split_by_ranges, split_document};

#[cfg(test)]
mod tests;
