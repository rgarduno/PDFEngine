//! Table data models and geometry representations according to ISO 32000-1 §14.8.4.
//!
//! Provides strongly typed representations for detected tabular structures,
//! grid cells, row/column spans, and multi-format export targets.

use crate::layout::geometry::Rect;

/// A single cell within a reconstructed table grid.
#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    /// 0-indexed row position within the table.
    pub row_idx: usize,
    /// 0-indexed column position within the table.
    pub col_idx: usize,
    /// Number of rows spanned by this cell (default 1).
    pub row_span: usize,
    /// Number of columns spanned by this cell (default 1).
    pub col_span: usize,
    /// Spatial bounding box rectangle in PDF page coordinates.
    pub bbox: Rect,
    /// Reconstructed textual content within the cell boundaries.
    pub text: String,
    /// Flag indicating whether this cell belongs to a table header row.
    pub is_header: bool,
}

impl TableCell {
    /// Creates a new table cell with default 1x1 span.
    pub fn new(row_idx: usize, col_idx: usize, bbox: Rect, text: impl Into<String>) -> Self {
        Self {
            row_idx,
            col_idx,
            row_span: 1,
            col_span: 1,
            bbox,
            text: text.into(),
            is_header: false,
        }
    }
}

/// A structured table detected on a document page.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedTable {
    /// 0-indexed identifier for the table on the page.
    pub table_idx: usize,
    /// 1-indexed page number containing this table.
    pub page_number: usize,
    /// Enclosing bounding box of the entire table.
    pub bbox: Rect,
    /// Total number of rows in the table grid.
    pub row_count: usize,
    /// Total number of columns in the table grid.
    pub col_count: usize,
    /// All individual cells belonging to the table.
    pub cells: Vec<TableCell>,
    /// Header labels extracted from the first row (or empty if no explicit header).
    pub headers: Vec<String>,
    /// 2D matrix of text values organized by [row][col].
    pub rows: Vec<Vec<String>>,
}

impl DetectedTable {
    /// Returns the cell at the given (row, col) coordinates, if present.
    pub fn get_cell(&self, row_idx: usize, col_idx: usize) -> Option<&TableCell> {
        self.cells
            .iter()
            .find(|c| c.row_idx == row_idx && c.col_idx == col_idx)
    }

    /// Rebuilds the 2D rows matrix and headers from the contained cells.
    pub fn rebuild_matrix(&mut self) {
        if self.row_count == 0 || self.col_count == 0 {
            self.headers = Vec::new();
            self.rows = Vec::new();
            return;
        }

        let mut matrix = vec![vec![String::new(); self.col_count]; self.row_count];
        for cell in &self.cells {
            if cell.row_idx < self.row_count && cell.col_idx < self.col_count {
                matrix[cell.row_idx][cell.col_idx] = cell.text.clone();
            }
        }

        if !matrix.is_empty() {
            self.headers = matrix[0].clone();
            self.rows = matrix[1..].to_vec();
        } else {
            self.headers = Vec::new();
            self.rows = Vec::new();
        }
    }
}

/// Supported export serialization formats for detected tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableExportFormat {
    /// RFC 4180 compliant Comma-Separated Values.
    Csv,
    /// Structured JSON schema with headers, rows, and cell bounding boxes.
    Json,
    /// GitHub Flavored Markdown table syntax.
    Markdown,
    /// Standard HTML `<table>` element with semantic `<thead>` and `<tbody>`.
    Html,
}
