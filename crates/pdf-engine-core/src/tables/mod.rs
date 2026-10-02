//! Table reconstruction, cell geometry analysis, and multi-format semantic extraction.
//!
//! Provides vector lattice detection, borderless layout analysis, cell text mapping,
//! and RFC 4180 CSV, JSON, Markdown, and HTML exports.

pub mod detector;
pub mod export;
pub mod types;

#[cfg(test)]
mod tests;

pub use detector::{detect_borderless_tables, detect_lattice_tables, detect_tables};
pub use export::{export_table, export_to_csv, export_to_html, export_to_json, export_to_markdown};
pub use types::{DetectedTable, TableCell, TableExportFormat};
