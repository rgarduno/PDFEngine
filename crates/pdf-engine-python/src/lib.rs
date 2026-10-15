//! Python bindings for the PDFEngine core using PyO3.
//!
//! Exposes high-level document loading, page scene graph inspection,
//! and in-place surgical paragraph editing to Python and FastAPI backends.
#![allow(clippy::too_many_arguments)]

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::BTreeMap;
use std::fs;

use pdf_engine_core::annots::{AnnotationSubtype, LinkAction, ShapeKind, ShapeStyle, StampType};
use pdf_engine_core::cos::{ObjectId, PdfDocument, PdfObject, PdfStream};
use pdf_engine_core::editor::{
    FillSignKind, FillSignMark, FillSignSpec, ParagraphStyle, SurgicalEditor, TextBoxSpec,
};
use pdf_engine_core::fonts::{resolve_page_fonts, FontMetrics, ResolvedFont};
use pdf_engine_core::layout::geometry::Rect;
use pdf_engine_core::layout::{LayoutReconstructor, ParagraphBlock, TextAlignment};
use pdf_engine_core::redact::{RedactionConfig, RedactionPattern, RedactionSummary};
use pdf_engine_core::security::{
    DigitalSignatureConfig, EncryptionOptions, EncryptionRevision, PdfPermissions,
    VerifiedSignature,
};
use pdf_engine_core::stream::{
    build_ast_from_operations, serialize_ast, ContentAst, ContentStreamTokenizer, Operation,
};
use pdf_engine_core::watermark::{
    ImageWatermarkConfig, PaginationConfig, PaginationPosition, TextWatermarkConfig,
    WatermarkPlacement,
};

/// High-level representation of an extracted paragraph block in Python.
#[pyclass(name = "Paragraph")]
#[derive(Debug, Clone)]
pub struct PyParagraph {
    #[pyo3(get)]
    pub id: usize,
    #[pyo3(get)]
    pub text: String,
    #[pyo3(get)]
    pub min_x: f64,
    #[pyo3(get)]
    pub min_y: f64,
    #[pyo3(get)]
    pub max_x: f64,
    #[pyo3(get)]
    pub max_y: f64,
    #[pyo3(get)]
    pub alignment: String,
    #[pyo3(get)]
    pub leading: f64,
    #[pyo3(get)]
    pub line_count: usize,
    /// Visual size in page points. Zero when the paragraph has no span.
    #[pyo3(get)]
    pub font_size: f64,
    /// CSS family. Empty when the face is unknown.
    #[pyo3(get)]
    pub font_family: String,
    #[pyo3(get)]
    pub font_weight: u16,
    /// `"normal"` or `"italic"`.
    #[pyo3(get)]
    pub font_style: String,
}

#[pymethods]
impl PyParagraph {
    fn __repr__(&self) -> String {
        format!(
            "<Paragraph id={} lines={} align='{}' text='{:.30}...'>",
            self.id,
            self.line_count,
            self.alignment,
            self.text.replace('\n', " ")
        )
    }

    /// Returns bounding box coordinates as a dictionary.
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_x, self.min_y, self.max_x, self.max_y)
    }
}

/// High-level representation of an extracted Image XObject on a page in Python.
#[pyclass(name = "ImageInfo")]
#[derive(Debug, Clone)]
pub struct PyImageInfo {
    #[pyo3(get)]
    pub id: u32,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub width_px: u32,
    #[pyo3(get)]
    pub height_px: u32,
    #[pyo3(get)]
    pub color_space: String,
    #[pyo3(get)]
    pub bits_per_component: u32,
    #[pyo3(get)]
    pub filter: Option<String>,
    #[pyo3(get)]
    pub byte_size: usize,
    #[pyo3(get)]
    pub min_x: f64,
    #[pyo3(get)]
    pub min_y: f64,
    #[pyo3(get)]
    pub max_x: f64,
    #[pyo3(get)]
    pub max_y: f64,
}

#[pymethods]
impl PyImageInfo {
    fn __repr__(&self) -> String {
        format!(
            "<ImageInfo id={} name='{}' size={}x{} cs='{}' bbox=({:.1}, {:.1}, {:.1}, {:.1})>",
            self.id,
            self.name,
            self.width_px,
            self.height_px,
            self.color_space,
            self.min_x,
            self.min_y,
            self.max_x,
            self.max_y
        )
    }

    /// Returns spatial bounding box coordinates as a tuple (min_x, min_y, max_x, max_y).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_x, self.min_y, self.max_x, self.max_y)
    }
}

/// High-level representation of an interactive AcroForm field in Python.
#[pyclass(name = "FormField")]
#[derive(Debug, Clone)]
pub struct PyFormField {
    #[pyo3(get)]
    pub id: u32,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub alt_name: Option<String>,
    #[pyo3(get)]
    pub field_type: String,
    #[pyo3(get)]
    pub value: String,
    #[pyo3(get)]
    pub default_value: Option<String>,
    #[pyo3(get)]
    pub min_x: f64,
    #[pyo3(get)]
    pub min_y: f64,
    #[pyo3(get)]
    pub max_x: f64,
    #[pyo3(get)]
    pub max_y: f64,
    #[pyo3(get)]
    pub page_number: usize,
    #[pyo3(get)]
    pub options: Vec<String>,
    #[pyo3(get)]
    pub is_read_only: bool,
    #[pyo3(get)]
    pub is_required: bool,
    #[pyo3(get)]
    pub is_multiline: bool,
    #[pyo3(get)]
    pub max_length: Option<usize>,
}

#[pymethods]
impl PyFormField {
    fn __repr__(&self) -> String {
        format!(
            "<FormField id={} name='{}' type='{}' value='{}' page={} bbox=({:.1}, {:.1}, {:.1}, {:.1})>",
            self.id, self.name, self.field_type, self.value, self.page_number,
            self.min_x, self.min_y, self.max_x, self.max_y
        )
    }

    /// Returns spatial bounding box coordinates as a tuple (min_x, min_y, max_x, max_y).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_x, self.min_y, self.max_x, self.max_y)
    }
}

/// A detected form field candidate before being converted into an interactive AcroForm widget.
#[pyclass(name = "DetectedFormFieldCandidate")]
#[derive(Debug, Clone)]
pub struct PyDetectedFormFieldCandidate {
    #[pyo3(get)]
    pub candidate_id: usize,
    #[pyo3(get)]
    pub field_type: String,
    #[pyo3(get)]
    pub suggested_name: String,
    #[pyo3(get)]
    pub label: Option<String>,
    #[pyo3(get)]
    pub min_x: f64,
    #[pyo3(get)]
    pub min_y: f64,
    #[pyo3(get)]
    pub max_x: f64,
    #[pyo3(get)]
    pub max_y: f64,
    #[pyo3(get)]
    pub confidence: f64,
}

#[pymethods]
impl PyDetectedFormFieldCandidate {
    #[new]
    #[pyo3(signature = (
        candidate_id=1,
        field_type="Text".to_string(),
        suggested_name="Field".to_string(),
        label=None,
        min_x=0.0,
        min_y=0.0,
        max_x=100.0,
        max_y=20.0,
        confidence=1.0
    ))]
    pub fn new(
        candidate_id: usize,
        field_type: String,
        suggested_name: String,
        label: Option<String>,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        confidence: f64,
    ) -> Self {
        Self {
            candidate_id,
            field_type,
            suggested_name,
            label,
            min_x,
            min_y,
            max_x,
            max_y,
            confidence,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "<DetectedFormFieldCandidate id={} type='{}' name='{}' label={:?} confidence={:.2} bbox=({:.1}, {:.1}, {:.1}, {:.1})>",
            self.candidate_id,
            self.field_type,
            self.suggested_name,
            self.label,
            self.confidence,
            self.min_x,
            self.min_y,
            self.max_x,
            self.max_y
        )
    }

    /// Returns spatial bounding box coordinates as a tuple (min_x, min_y, max_x, max_y).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_x, self.min_y, self.max_x, self.max_y)
    }
}

impl PyDetectedFormFieldCandidate {
    pub fn from_core(c: pdf_engine_core::forms::DetectedFormFieldCandidate) -> Self {
        Self {
            candidate_id: c.candidate_id,
            field_type: c.field_type.as_str().to_string(),
            suggested_name: c.suggested_name,
            label: c.label,
            min_x: c.rect.min_x,
            min_y: c.rect.min_y,
            max_x: c.rect.max_x,
            max_y: c.rect.max_y,
            confidence: c.confidence,
        }
    }

    pub fn to_core(&self) -> pdf_engine_core::forms::DetectedFormFieldCandidate {
        let ft = match self.field_type.to_lowercase().as_str() {
            "checkbox" | "btn" => pdf_engine_core::forms::FormFieldType::Checkbox,
            "signature" | "sig" => pdf_engine_core::forms::FormFieldType::Signature,
            "choice" | "ch" => pdf_engine_core::forms::FormFieldType::Choice,
            _ => pdf_engine_core::forms::FormFieldType::Text,
        };
        pdf_engine_core::forms::DetectedFormFieldCandidate {
            candidate_id: self.candidate_id,
            field_type: ft,
            suggested_name: self.suggested_name.clone(),
            label: self.label.clone(),
            rect: pdf_engine_core::layout::geometry::Rect::new(
                self.min_x, self.min_y, self.max_x, self.max_y,
            ),
            confidence: self.confidence,
        }
    }
}

/// High-level representation of a PDF annotation in Python.
#[pyclass(name = "Annotation")]
#[derive(Debug, Clone)]
pub struct PyAnnotation {
    #[pyo3(get)]
    pub id: u32,
    #[pyo3(get)]
    pub page_index: usize,
    #[pyo3(get)]
    pub page_number: usize,
    #[pyo3(get)]
    pub subtype: String,
    #[pyo3(get)]
    pub min_x: f64,
    #[pyo3(get)]
    pub min_y: f64,
    #[pyo3(get)]
    pub max_x: f64,
    #[pyo3(get)]
    pub max_y: f64,
    #[pyo3(get)]
    pub color: Option<Vec<f64>>,
    #[pyo3(get)]
    pub opacity: f64,
    #[pyo3(get)]
    pub contents: Option<String>,
    #[pyo3(get)]
    pub link_type: Option<String>,
    #[pyo3(get)]
    pub link_uri: Option<String>,
    #[pyo3(get)]
    pub link_target_page: Option<usize>,
    #[pyo3(get)]
    pub stamp_type: Option<String>,
    #[pyo3(get)]
    pub date_str: Option<String>,
    #[pyo3(get)]
    pub border_width: f64,
    #[pyo3(get)]
    pub fill_color: Option<Vec<f64>>,
    #[pyo3(get)]
    pub points: Vec<(f64, f64)>,
    #[pyo3(get)]
    pub line_ending: Option<String>,
}

#[pymethods]
impl PyAnnotation {
    fn __repr__(&self) -> String {
        format!(
            "<Annotation id={} type='{}' page={} bbox=({:.1}, {:.1}, {:.1}, {:.1})>",
            self.id, self.subtype, self.page_number, self.min_x, self.min_y, self.max_x, self.max_y
        )
    }

    /// Returns spatial bounding box coordinates as a tuple (min_x, min_y, max_x, max_y).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_x, self.min_y, self.max_x, self.max_y)
    }
}

impl PyAnnotation {
    pub(crate) fn from_core(a: pdf_engine_core::annots::Annotation) -> Self {
        let (link_type, link_uri, link_target_page) = match a.link_action {
            Some(LinkAction::Uri(uri)) => (Some("URI".to_string()), Some(uri), None),
            Some(LinkAction::GoTo(target_idx)) => {
                (Some("GoTo".to_string()), None, Some(target_idx + 1))
            }
            None => (None, None, None),
        };

        let stamp_type = a.stamp_type.map(|s| s.text());

        PyAnnotation {
            id: a.id.number,
            page_index: a.page_index,
            page_number: a.page_index + 1,
            subtype: a.subtype.as_pdf_name().to_string(),
            min_x: a.rect.min_x,
            min_y: a.rect.min_y,
            max_x: a.rect.max_x,
            max_y: a.rect.max_y,
            color: a.color.map(|c| c.to_vec()),
            opacity: a.opacity,
            contents: a.contents,
            link_type,
            link_uri,
            link_target_page,
            stamp_type,
            date_str: a.date_str,
            border_width: a.border_width,
            fill_color: a.fill_color.map(|color| color.to_vec()),
            points: a.points.iter().map(|point| (point[0], point[1])).collect(),
            line_ending: a.line_ending,
        }
    }
}

/// High-level representation of an applied redaction pass in Python.
#[pyclass(name = "RedactionSummary")]
#[derive(Debug, Clone)]
pub struct PyRedactionSummary {
    #[pyo3(get)]
    pub page_index: usize,
    #[pyo3(get)]
    pub page_number: usize,
    #[pyo3(get)]
    pub purged_glyphs_count: usize,
    #[pyo3(get)]
    pub modified_blocks_count: usize,
    #[pyo3(get)]
    pub blackout_boxes_count: usize,
    #[pyo3(get)]
    pub pruned_annotations_count: usize,
    #[pyo3(get)]
    pub applied_rects: Vec<(f64, f64, f64, f64)>,
}

#[pymethods]
impl PyRedactionSummary {
    fn __repr__(&self) -> String {
        format!(
            "<RedactionSummary page={} purged_glyphs={} blackout_boxes={} pruned_annots={}>",
            self.page_number,
            self.purged_glyphs_count,
            self.blackout_boxes_count,
            self.pruned_annotations_count
        )
    }
}

impl PyRedactionSummary {
    pub(crate) fn from_core(s: RedactionSummary) -> Self {
        let rects = s
            .applied_rects
            .into_iter()
            .map(|r| (r.min_x, r.min_y, r.max_x, r.max_y))
            .collect();

        PyRedactionSummary {
            page_index: s.page_index,
            page_number: s.page_index + 1,
            purged_glyphs_count: s.purged_glyphs_count,
            modified_blocks_count: s.modified_blocks_count,
            blackout_boxes_count: s.blackout_boxes_count,
            pruned_annotations_count: s.pruned_annotations_count,
            applied_rects: rects,
        }
    }
}

/// Bits stored in the encryption dictionary `/P` entry.
/// After a user password opens the file, this process refuses print, copy, and
/// edit when those bits are clear. The owner password is unrestricted. A user
/// password whose enforced bits are all set is not told apart from the owner.
/// Bit 12 is stored and is not a second raster. Bit 10 is not a separate extract
/// path. Saving the decrypted file drops `/P`. A page already returned still
/// holds the paragraphs it reconstructed. The bit positions follow ISO 32000-1
/// §7.6.3.2 Table 22.
#[pyclass(name = "PdfPermissions")]
#[derive(Debug, Clone)]
pub struct PyPdfPermissions {
    #[pyo3(get, set)]
    pub print_low_res: bool,
    #[pyo3(get, set)]
    pub print_high_res: bool,
    #[pyo3(get, set)]
    pub modify_contents: bool,
    #[pyo3(get, set)]
    pub copy_extract: bool,
    #[pyo3(get, set)]
    pub modify_annotations: bool,
    #[pyo3(get, set)]
    pub fill_forms: bool,
    #[pyo3(get, set)]
    pub accessibility_extract: bool,
    #[pyo3(get, set)]
    pub assemble_document: bool,
}

#[pymethods]
impl PyPdfPermissions {
    #[new]
    #[pyo3(signature = (
        print_low_res=true,
        print_high_res=true,
        modify_contents=true,
        copy_extract=true,
        modify_annotations=true,
        fill_forms=true,
        accessibility_extract=true,
        assemble_document=true
    ))]
    pub fn new(
        print_low_res: bool,
        print_high_res: bool,
        modify_contents: bool,
        copy_extract: bool,
        modify_annotations: bool,
        fill_forms: bool,
        accessibility_extract: bool,
        assemble_document: bool,
    ) -> Self {
        Self {
            print_low_res,
            print_high_res,
            modify_contents,
            copy_extract,
            modify_annotations,
            fill_forms,
            accessibility_extract,
            assemble_document,
        }
    }

    #[staticmethod]
    pub fn read_only() -> Self {
        Self {
            print_low_res: true,
            print_high_res: false,
            modify_contents: false,
            copy_extract: false,
            modify_annotations: false,
            fill_forms: false,
            accessibility_extract: true,
            assemble_document: false,
        }
    }

    #[staticmethod]
    pub fn full_access() -> Self {
        Self::new(true, true, true, true, true, true, true, true)
    }

    pub fn to_p_value(&self) -> i32 {
        self.to_core().to_p_value()
    }
}

impl PyPdfPermissions {
    pub fn to_core(&self) -> PdfPermissions {
        PdfPermissions {
            print_low_res: self.print_low_res,
            print_high_res: self.print_high_res,
            modify_contents: self.modify_contents,
            copy_extract: self.copy_extract,
            modify_annotations: self.modify_annotations,
            fill_forms: self.fill_forms,
            accessibility_extract: self.accessibility_extract,
            assemble_document: self.assemble_document,
        }
    }

    pub fn from_core(p: PdfPermissions) -> Self {
        Self {
            print_low_res: p.print_low_res,
            print_high_res: p.print_high_res,
            modify_contents: p.modify_contents,
            copy_extract: p.copy_extract,
            modify_annotations: p.modify_annotations,
            fill_forms: p.fill_forms,
            accessibility_extract: p.accessibility_extract,
            assemble_document: p.assemble_document,
        }
    }
}

/// Read-back of an embedded signature dictionary and its byte-range digest check.
#[pyclass(name = "VerifiedSignature")]
#[derive(Debug, Clone)]
pub struct PyVerifiedSignature {
    #[pyo3(get)]
    pub field_name: String,
    #[pyo3(get)]
    pub signer_name: String,
    #[pyo3(get)]
    pub reason: String,
    #[pyo3(get)]
    pub location: String,
    #[pyo3(get)]
    pub date: String,
    #[pyo3(get)]
    pub sub_filter: String,
    #[pyo3(get)]
    pub byte_range: Vec<usize>,
    #[pyo3(get)]
    pub contents_hex: String,
    #[pyo3(get)]
    pub byte_range_valid: bool,
    #[pyo3(get)]
    pub rect: [f64; 4],
    #[pyo3(get)]
    pub page_number: usize,
}

#[pymethods]
impl PyVerifiedSignature {
    fn __repr__(&self) -> String {
        format!(
            "<VerifiedSignature field='{}' signer='{}' valid={}>",
            self.field_name, self.signer_name, self.byte_range_valid
        )
    }
}

impl PyVerifiedSignature {
    pub fn from_core(s: VerifiedSignature) -> Self {
        Self {
            field_name: s.field_name,
            signer_name: s.signer_name,
            reason: s.reason,
            location: s.location,
            date: s.date,
            sub_filter: s.sub_filter,
            byte_range: s.byte_range,
            contents_hex: s.contents_hex,
            byte_range_valid: s.byte_range_valid,
            rect: s.rect,
            page_number: s.page_number,
        }
    }
}

/// High-level representation of an extracted table cell.
#[pyclass(name = "TableCell")]
#[derive(Debug, Clone)]
pub struct PyTableCell {
    #[pyo3(get)]
    pub row: usize,
    #[pyo3(get)]
    pub col: usize,
    #[pyo3(get)]
    pub row_span: usize,
    #[pyo3(get)]
    pub col_span: usize,
    #[pyo3(get)]
    pub text: String,
    #[pyo3(get)]
    pub is_header: bool,
    #[pyo3(get)]
    pub min_x: f64,
    #[pyo3(get)]
    pub min_y: f64,
    #[pyo3(get)]
    pub max_x: f64,
    #[pyo3(get)]
    pub max_y: f64,
}

#[pymethods]
impl PyTableCell {
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_x, self.min_y, self.max_x, self.max_y)
    }
}

/// High-level representation of a detected table on a page.
#[pyclass(name = "DetectedTable")]
#[derive(Debug, Clone)]
pub struct PyDetectedTable {
    #[pyo3(get)]
    pub table_idx: usize,
    #[pyo3(get)]
    pub page_number: usize,
    #[pyo3(get)]
    pub row_count: usize,
    #[pyo3(get)]
    pub col_count: usize,
    #[pyo3(get)]
    pub min_x: f64,
    #[pyo3(get)]
    pub min_y: f64,
    #[pyo3(get)]
    pub max_x: f64,
    #[pyo3(get)]
    pub max_y: f64,
    #[pyo3(get)]
    pub headers: Vec<String>,
    #[pyo3(get)]
    pub rows: Vec<Vec<String>>,
    #[pyo3(get)]
    pub cells: Vec<PyTableCell>,
}

#[pymethods]
impl PyDetectedTable {
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        (self.min_x, self.min_y, self.max_x, self.max_y)
    }

    pub fn to_csv(&self) -> String {
        let core_table = self.to_core();
        pdf_engine_core::tables::export_to_csv(&core_table)
    }

    pub fn to_json(&self) -> String {
        let core_table = self.to_core();
        pdf_engine_core::tables::export_to_json(&core_table)
    }

    pub fn to_markdown(&self) -> String {
        let core_table = self.to_core();
        pdf_engine_core::tables::export_to_markdown(&core_table)
    }

    pub fn to_html(&self) -> String {
        let core_table = self.to_core();
        pdf_engine_core::tables::export_to_html(&core_table)
    }
}

impl PyDetectedTable {
    pub fn from_core(t: pdf_engine_core::tables::DetectedTable) -> Self {
        let cells = t
            .cells
            .iter()
            .map(|c| PyTableCell {
                row: c.row_idx,
                col: c.col_idx,
                row_span: c.row_span,
                col_span: c.col_span,
                text: c.text.clone(),
                is_header: c.is_header,
                min_x: c.bbox.min_x,
                min_y: c.bbox.min_y,
                max_x: c.bbox.max_x,
                max_y: c.bbox.max_y,
            })
            .collect();

        Self {
            table_idx: t.table_idx,
            page_number: t.page_number,
            row_count: t.row_count,
            col_count: t.col_count,
            min_x: t.bbox.min_x,
            min_y: t.bbox.min_y,
            max_x: t.bbox.max_x,
            max_y: t.bbox.max_y,
            headers: t.headers,
            rows: t.rows,
            cells,
        }
    }

    pub fn to_core(&self) -> pdf_engine_core::tables::DetectedTable {
        let cells = self
            .cells
            .iter()
            .map(|c| pdf_engine_core::tables::TableCell {
                row_idx: c.row,
                col_idx: c.col,
                row_span: c.row_span,
                col_span: c.col_span,
                bbox: Rect::new(c.min_x, c.min_y, c.max_x, c.max_y),
                text: c.text.clone(),
                is_header: c.is_header,
            })
            .collect();

        pdf_engine_core::tables::DetectedTable {
            table_idx: self.table_idx,
            page_number: self.page_number,
            bbox: Rect::new(self.min_x, self.min_y, self.max_x, self.max_y),
            row_count: self.row_count,
            col_count: self.col_count,
            cells,
            headers: self.headers.clone(),
            rows: self.rows.clone(),
        }
    }
}

/// Statistics and compression metrics resulting from document optimization.
#[pyclass(name = "OptimizationStats")]
#[derive(Debug, Clone)]
pub struct PyOptimizationStats {
    #[pyo3(get)]
    pub original_size: usize,
    #[pyo3(get)]
    pub optimized_size: usize,
    #[pyo3(get)]
    pub bytes_saved: usize,
    #[pyo3(get)]
    pub compression_ratio_pct: f64,
    #[pyo3(get)]
    pub objects_removed: usize,
    #[pyo3(get)]
    pub streams_recompressed: usize,
    #[pyo3(get)]
    pub object_streams_created: usize,
    #[pyo3(get)]
    pub streams_deduplicated: usize,
}

#[pymethods]
impl PyOptimizationStats {
    fn __repr__(&self) -> String {
        format!(
            "<OptimizationStats original={} optimized={} saved={} ({:.1}%) objects_removed={} streams_recompressed={}>",
            self.original_size,
            self.optimized_size,
            self.bytes_saved,
            self.compression_ratio_pct,
            self.objects_removed,
            self.streams_recompressed
        )
    }

    /// Converts the statistics object to a Python dictionary.
    pub fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("original_size", self.original_size)?;
        dict.set_item("optimized_size", self.optimized_size)?;
        dict.set_item("bytes_saved", self.bytes_saved)?;
        dict.set_item("compression_ratio_pct", self.compression_ratio_pct)?;
        dict.set_item("objects_removed", self.objects_removed)?;
        dict.set_item("streams_recompressed", self.streams_recompressed)?;
        dict.set_item("object_streams_created", self.object_streams_created)?;
        dict.set_item("streams_deduplicated", self.streams_deduplicated)?;
        Ok(dict)
    }
}

impl PyOptimizationStats {
    pub fn from_core(stats: pdf_engine_core::ops::OptimizationStats) -> Self {
        Self {
            original_size: stats.original_size,
            optimized_size: stats.optimized_size,
            bytes_saved: stats.bytes_saved,
            compression_ratio_pct: stats.compression_ratio_pct,
            objects_removed: stats.objects_removed,
            streams_recompressed: stats.streams_recompressed,
            object_streams_created: stats.object_streams_created,
            streams_deduplicated: stats.streams_deduplicated,
        }
    }
}

/// Hierarchical bookmark / Table of Contents item in Python (`/Outlines`).
#[pyclass(name = "OutlineItem")]
#[derive(Debug, Clone)]
pub struct PyOutlineItem {
    #[pyo3(get, set)]
    pub title: String,
    #[pyo3(get, set)]
    pub page_number: Option<usize>,
    #[pyo3(get, set)]
    pub top: Option<f64>,
    #[pyo3(get, set)]
    pub left: Option<f64>,
    #[pyo3(get, set)]
    pub zoom: Option<f64>,
    #[pyo3(get, set)]
    pub uri: Option<String>,
    #[pyo3(get, set)]
    pub is_open: bool,
    #[pyo3(get, set)]
    pub bold: bool,
    #[pyo3(get, set)]
    pub italic: bool,
    #[pyo3(get, set)]
    pub color: Option<Vec<f64>>,
    #[pyo3(get, set)]
    pub children: Vec<PyOutlineItem>,
}

#[pymethods]
impl PyOutlineItem {
    #[new]
    #[pyo3(signature = (
        title,
        page_number=None,
        top=None,
        left=None,
        zoom=None,
        uri=None,
        is_open=true,
        bold=false,
        italic=false,
        color=None,
        children=None
    ))]
    pub fn new(
        title: String,
        page_number: Option<usize>,
        top: Option<f64>,
        left: Option<f64>,
        zoom: Option<f64>,
        uri: Option<String>,
        is_open: bool,
        bold: bool,
        italic: bool,
        color: Option<Vec<f64>>,
        children: Option<Vec<PyOutlineItem>>,
    ) -> Self {
        Self {
            title,
            page_number,
            top,
            left,
            zoom,
            uri,
            is_open,
            bold,
            italic,
            color,
            children: children.unwrap_or_default(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "<OutlineItem title='{}' page={:?} uri={:?} children={}>",
            self.title,
            self.page_number,
            self.uri,
            self.children.len()
        )
    }
}

impl PyOutlineItem {
    pub fn from_core(item: pdf_engine_core::ops::OutlineItem) -> Self {
        Self {
            title: item.title,
            page_number: item.page_number,
            top: item.top,
            left: item.left,
            zoom: item.zoom,
            uri: item.uri,
            is_open: item.is_open,
            bold: item.bold,
            italic: item.italic,
            color: item.color.map(|c| c.to_vec()),
            children: item
                .children
                .into_iter()
                .map(PyOutlineItem::from_core)
                .collect(),
        }
    }

    pub fn to_core(&self) -> pdf_engine_core::ops::OutlineItem {
        let color = self.color.as_ref().and_then(|c| {
            if c.len() == 3 {
                Some([c[0], c[1], c[2]])
            } else {
                None
            }
        });
        pdf_engine_core::ops::OutlineItem {
            title: self.title.clone(),
            page_number: self.page_number,
            top: self.top,
            left: self.left,
            zoom: self.zoom,
            uri: self.uri.clone(),
            is_open: self.is_open,
            bold: self.bold,
            italic: self.italic,
            color,
            children: self.children.iter().map(PyOutlineItem::to_core).collect(),
        }
    }
}

/// Metadata describing an embedded file attachment in a PDF document (`/Names /EmbeddedFiles` or `/FileAttachment`).
#[pyclass(name = "EmbeddedAttachmentInfo")]
#[derive(Debug, Clone)]
pub struct PyEmbeddedAttachmentInfo {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub filename: String,
    #[pyo3(get)]
    pub mime_type: String,
    #[pyo3(get)]
    pub size: usize,
    #[pyo3(get)]
    pub description: Option<String>,
    #[pyo3(get)]
    pub creation_date: Option<String>,
    #[pyo3(get)]
    pub modification_date: Option<String>,
    #[pyo3(get)]
    pub checksum_md5: Option<String>,
    #[pyo3(get)]
    pub checksum_sha256: String,
    #[pyo3(get)]
    pub source: String,
    #[pyo3(get)]
    pub page_number: Option<usize>,
}

#[pymethods]
impl PyEmbeddedAttachmentInfo {
    fn __repr__(&self) -> String {
        format!(
            "<EmbeddedAttachmentInfo name='{}' mime='{}' size={} sha256='{}'>",
            self.name, self.mime_type, self.size, self.checksum_sha256
        )
    }
}

impl PyEmbeddedAttachmentInfo {
    pub fn from_core(info: pdf_engine_core::ops::EmbeddedAttachmentInfo) -> Self {
        Self {
            name: info.name,
            filename: info.filename,
            mime_type: info.mime_type,
            size: info.size,
            description: info.description,
            creation_date: info.creation_date,
            modification_date: info.modification_date,
            checksum_md5: info.checksum_md5,
            checksum_sha256: info.checksum_sha256,
            source: info.source,
            page_number: info.page_number,
        }
    }
}

/// Extracted embedded file attachment including metadata and raw uncompressed bytes.
#[pyclass(name = "EmbeddedAttachment")]
#[derive(Debug, Clone)]
pub struct PyEmbeddedAttachment {
    #[pyo3(get)]
    pub info: PyEmbeddedAttachmentInfo,
    #[pyo3(get)]
    pub data: Vec<u8>,
}

#[pymethods]
impl PyEmbeddedAttachment {
    fn __repr__(&self) -> String {
        format!(
            "<EmbeddedAttachment name='{}' mime='{}' bytes={}>",
            self.info.name,
            self.info.mime_type,
            self.data.len()
        )
    }
}

impl PyEmbeddedAttachment {
    pub fn from_core(att: pdf_engine_core::ops::EmbeddedAttachment) -> Self {
        Self {
            info: PyEmbeddedAttachmentInfo::from_core(att.info),
            data: att.data,
        }
    }
}

/// Represents an Optional Content Group (`/Type /OCG` — PDF Layer, ISO 32000-1 §8.11) in Python.
#[pyclass(name = "OptionalContentLayer")]
#[derive(Debug, Clone)]
pub struct PyOptionalContentLayer {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub object_number: u32,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub visible: bool,
    #[pyo3(get)]
    pub intent: Vec<String>,
    #[pyo3(get)]
    pub locked: bool,
}

#[pymethods]
impl PyOptionalContentLayer {
    fn __repr__(&self) -> String {
        format!(
            "<OptionalContentLayer id='{}' name='{}' visible={} locked={} intent={:?}>",
            self.id, self.name, self.visible, self.locked, self.intent
        )
    }

    /// Converts the layer descriptor to a Python dictionary.
    pub fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("id", &self.id)?;
        dict.set_item("object_number", self.object_number)?;
        dict.set_item("name", &self.name)?;
        dict.set_item("visible", self.visible)?;
        dict.set_item("intent", &self.intent)?;
        dict.set_item("locked", self.locked)?;
        Ok(dict)
    }
}

impl PyOptionalContentLayer {
    pub fn from_core(layer: pdf_engine_core::ops::OptionalContentLayer) -> Self {
        Self {
            id: layer.id,
            object_number: layer.object_number,
            name: layer.name,
            visible: layer.visible,
            intent: layer.intent,
            locked: layer.locked,
        }
    }
}

/// Summary report returned after executing a permanent hard-flatten (burn-in) operation.
#[pyclass(name = "HardFlattenReport")]
#[derive(Debug, Clone)]
pub struct PyHardFlattenReport {
    #[pyo3(get)]
    pub pages_flattened: usize,
    #[pyo3(get)]
    pub forms_flattened: usize,
    #[pyo3(get)]
    pub annotations_flattened: usize,
    #[pyo3(get)]
    pub appearance_streams_compiled: usize,
    #[pyo3(get)]
    pub objects_purged: usize,
}

#[pymethods]
impl PyHardFlattenReport {
    fn __repr__(&self) -> String {
        format!(
            "<HardFlattenReport pages={} forms={} annotations={} ap_streams={} purged={}>",
            self.pages_flattened,
            self.forms_flattened,
            self.annotations_flattened,
            self.appearance_streams_compiled,
            self.objects_purged
        )
    }

    /// Converts the hard-flatten report to a Python dictionary.
    pub fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("pages_flattened", self.pages_flattened)?;
        dict.set_item("forms_flattened", self.forms_flattened)?;
        dict.set_item("annotations_flattened", self.annotations_flattened)?;
        dict.set_item(
            "appearance_streams_compiled",
            self.appearance_streams_compiled,
        )?;
        dict.set_item("objects_purged", self.objects_purged)?;
        Ok(dict)
    }
}

impl PyHardFlattenReport {
    pub fn from_core(report: pdf_engine_core::ops::HardFlattenReport) -> Self {
        Self {
            pages_flattened: report.pages_flattened,
            forms_flattened: report.forms_flattened,
            annotations_flattened: report.annotations_flattened,
            appearance_streams_compiled: report.appearance_streams_compiled,
            objects_purged: report.objects_purged,
        }
    }
}

/// Represents a single page within a PDF document in Python.
#[pyclass(name = "Page")]
pub struct PyPage {
    #[pyo3(get)]
    pub page_number: usize,
    pub page_id: ObjectId,
    pub contents_id: Option<ObjectId>,
    pub ast: ContentAst,
    pub paragraphs: Vec<ParagraphBlock>,
    pub metrics: FontMetrics,
    faces: BTreeMap<String, ResolvedFont>,
    hidden_ocg_props: std::collections::HashSet<String>,
    /// Copied from the session when this page object is built. A page already
    /// returned keeps the value it had then.
    text_extract_allowed: bool,
}

#[pymethods]
impl PyPage {
    /// Returns the list of reconstructed paragraph blocks on this page.
    pub fn get_paragraphs(&self) -> PyResult<Vec<PyParagraph>> {
        if !self.text_extract_allowed {
            return Err(permission_denied());
        }
        Ok(self
            .paragraphs
            .iter()
            .map(|p| {
                let align_str = match p.alignment {
                    TextAlignment::Left => "left",
                    TextAlignment::Center => "center",
                    TextAlignment::Right => "right",
                    TextAlignment::Justified => "justified",
                };
                let (font_size, font_family, font_weight, font_style) = face_style(p, &self.faces);
                PyParagraph {
                    id: p.id,
                    text: p.text(),
                    min_x: p.bbox.min_x,
                    min_y: p.bbox.min_y,
                    max_x: p.bbox.max_x,
                    max_y: p.bbox.max_y,
                    alignment: align_str.to_string(),
                    leading: p.leading,
                    line_count: p.lines.len(),
                    font_size,
                    font_family,
                    font_weight,
                    font_style,
                }
            })
            .collect())
    }

    /// Surgically edits a paragraph in-place on this page without modifying surrounding graphics.
    pub fn edit_paragraph(&mut self, paragraph_id: usize, new_text: &str) -> PyResult<()> {
        let target_block = self
            .paragraphs
            .iter()
            .find(|p| p.id == paragraph_id)
            .cloned()
            .ok_or_else(|| {
                PyValueError::new_err(format!("Paragraph id {} not found on page", paragraph_id))
            })?;

        SurgicalEditor::edit_paragraph(&mut self.ast, &target_block, new_text, &self.metrics)
            .map_err(|e| PyRuntimeError::new_err(format!("Surgical edit failed: {}", e)))?;

        self.paragraphs = reconstruct_layout(
            &self.ast,
            &self.metrics,
            &self.faces,
            &self.hidden_ocg_props,
        )?;

        Ok(())
    }

    /// Serializes the page content stream into raw bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        serialize_ast(&self.ast)
    }

    /// Returns a JSON-formatted SceneGraph of the page layout.
    pub fn scenegraph_json(&self) -> PyResult<String> {
        let paragraphs = self.get_paragraphs()?;
        let mut json = String::from("{\"page\":");
        json.push_str(&self.page_number.to_string());
        json.push_str(",\"paragraphs\":[");

        for (i, p) in paragraphs.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "{{\"id\":{},\"text\":{:?},\"bbox\":{{\"min_x\":{:.2},\"min_y\":{:.2},\"max_x\":{:.2},\"max_y\":{:.2}}},\"alignment\":\"{}\",\"leading\":{:.2},\"lines\":{},\"font_size\":{:.2},\"font_family\":{:?},\"font_weight\":{},\"font_style\":{:?}}}",
                p.id,
                p.text,
                p.min_x,
                p.min_y,
                p.max_x,
                p.max_y,
                p.alignment,
                p.leading,
                p.line_count,
                p.font_size,
                p.font_family,
                p.font_weight,
                p.font_style
            ));
        }

        json.push_str("]}");
        Ok(json)
    }
}

/// Primary PDF Document controller exposed to Python.
#[pyclass(name = "Document")]
pub struct PyPdfDocument {
    doc: PdfDocument,
    page_ids: Vec<ObjectId>,
    active_pages: Vec<PyPage>,
}

/// Tokenizes the decoded page content.
///
/// Operators are read from the inflated stream. Reading the stored bytes of a
/// Flate stream treats compressed data as operators and can sit on one delimiter.
fn face_style(
    block: &ParagraphBlock,
    faces: &BTreeMap<String, ResolvedFont>,
) -> (f64, String, u16, String) {
    match faces.get(&block.font_resource()) {
        Some(face) => (
            block.rendered_size(),
            face.family.clone(),
            face.weight,
            face.style.clone(),
        ),
        None => (
            block.rendered_size(),
            String::new(),
            400,
            "normal".to_string(),
        ),
    }
}

fn reconstruct_layout(
    ast: &ContentAst,
    metrics: &FontMetrics,
    faces: &BTreeMap<String, ResolvedFont>,
    hidden_ocg_props: &std::collections::HashSet<String>,
) -> PyResult<Vec<ParagraphBlock>> {
    LayoutReconstructor::new(ast)
        .with_font("F1", metrics.clone())
        .with_resolved(faces)
        .with_hidden_ocg_props(hidden_ocg_props.clone())
        .reconstruct()
        .map_err(|e| PyRuntimeError::new_err(format!("Layout reconstruction failed: {}", e)))
}

fn tokenize_decoded_page(doc: &mut PdfDocument, page_id: ObjectId) -> PyResult<Vec<Operation>> {
    let decoded = doc
        .get_page_content_bytes(page_id)
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to load stream: {}", e)))?;
    let mut tokenizer = ContentStreamTokenizer::new(&decoded);
    Ok(tokenizer.tokenize_all().unwrap_or_default())
}

impl PyPdfDocument {
    /// Internal factory reconstructing active page scene graphs from a core PdfDocument.
    pub fn from_doc(mut doc: PdfDocument) -> PyResult<Self> {
        let page_ids = doc
            .get_pages()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to resolve pages: {}", e)))?;

        let mut active_pages = Vec::with_capacity(page_ids.len());
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
        let text_extract_allowed = doc.allows_copy();

        for (idx, &page_id) in page_ids.iter().enumerate() {
            let faces = resolve_page_fonts(&mut doc, page_id)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to resolve fonts: {}", e)))?;
            let hidden_ocg_props =
                pdf_engine_core::ops::resolve_page_hidden_ocg_props(&mut doc, page_id)
                    .unwrap_or_default();
            let page_obj = doc
                .get_object(page_id)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to load page: {}", e)))?;

            let (contents_id, ast, paragraphs) = if let Some(dict) = page_obj.as_dict() {
                let c_ref = dict.get("Contents").and_then(|c| c.as_reference());
                if let Some(contents_ref) = c_ref {
                    let contents_obj = doc.get_object(contents_ref).map_err(|e| {
                        PyRuntimeError::new_err(format!("Failed to load stream: {}", e))
                    })?;

                    if let PdfObject::Stream(_) = contents_obj {
                        let ops = tokenize_decoded_page(&mut doc, page_id)?;
                        let ast = build_ast_from_operations(ops);
                        let paragraphs =
                            reconstruct_layout(&ast, &metrics, &faces, &hidden_ocg_props)?;
                        (Some(contents_ref), ast, paragraphs)
                    } else {
                        (None, ContentAst::new(), Vec::new())
                    }
                } else {
                    (None, ContentAst::new(), Vec::new())
                }
            } else {
                (None, ContentAst::new(), Vec::new())
            };

            active_pages.push(PyPage {
                page_number: idx + 1,
                page_id,
                contents_id,
                ast,
                paragraphs,
                metrics: metrics.clone(),
                faces,
                hidden_ocg_props,
                text_extract_allowed,
            });
        }

        Ok(Self {
            doc,
            page_ids,
            active_pages,
        })
    }

    /// Reloads all active PyPage scene graphs to reflect recent stream mutations or flatten operations.
    pub(crate) fn reload_active_pages(&mut self) -> PyResult<()> {
        let page_ids = self.page_ids.clone();
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
        let text_extract_allowed = self.doc.allows_copy();
        let mut reloaded_pages = Vec::with_capacity(page_ids.len());

        for (idx, &page_id) in page_ids.iter().enumerate() {
            let faces = resolve_page_fonts(&mut self.doc, page_id)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to resolve fonts: {}", e)))?;
            let hidden_ocg_props =
                pdf_engine_core::ops::resolve_page_hidden_ocg_props(&mut self.doc, page_id)
                    .unwrap_or_default();
            if let Ok(page_obj) = self.doc.get_object(page_id) {
                if let Some(dict) = page_obj.as_dict() {
                    let contents_id = dict.get("Contents").and_then(|c| c.as_reference());
                    let (ast, paragraphs) = if let Some(c_ref) = contents_id {
                        if let Ok(PdfObject::Stream(_)) = self.doc.get_object(c_ref) {
                            let ops =
                                tokenize_decoded_page(&mut self.doc, page_id).unwrap_or_default();
                            let ast = build_ast_from_operations(ops);
                            let paragraphs =
                                reconstruct_layout(&ast, &metrics, &faces, &hidden_ocg_props)?;
                            (ast, paragraphs)
                        } else {
                            (ContentAst::new(), Vec::new())
                        }
                    } else {
                        (ContentAst::new(), Vec::new())
                    };

                    reloaded_pages.push(PyPage {
                        page_number: idx + 1,
                        page_id,
                        contents_id,
                        ast,
                        paragraphs,
                        metrics: metrics.clone(),
                        faces,
                        hidden_ocg_props,
                        text_extract_allowed,
                    });
                }
            }
        }
        self.active_pages = reloaded_pages;
        Ok(())
    }

    /// Rebuilds page handles after a composition changes the page list.
    fn refresh_handles(&mut self) -> PyResult<()> {
        let updated = Self::from_doc(self.doc.clone())?;
        self.doc = updated.doc;
        self.page_ids = updated.page_ids;
        self.active_pages = updated.active_pages;
        Ok(())
    }
}

#[pymethods]
impl PyPdfDocument {
    /// Loads a PDF document from a filesystem file path.
    #[staticmethod]
    pub fn load(path: &str) -> PyResult<Self> {
        let bytes = fs::read(path)
            .map_err(|e| PyIOError::new_err(format!("Failed to read file '{}': {}", path, e)))?;
        Self::from_bytes(&bytes)
    }

    /// Loads a PDF document from an in-memory byte slice.
    #[staticmethod]
    pub fn from_bytes(bytes: &[u8]) -> PyResult<Self> {
        let doc = PdfDocument::load(bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("PDF parse error: {}", e)))?;
        Self::from_doc(doc)
    }

    /// Returns the total number of pages in the document.
    pub fn page_count(&self) -> usize {
        self.page_ids.len()
    }

    /// Recognizes single-image pages and appends invisible searchable text.
    ///
    /// `language` is a tesseract traineddata name. `None` uses `eng`.
    /// Returns `(pages_seen, pages_recognized, words_inserted)`.
    #[pyo3(signature = (language=None))]
    pub fn add_searchable_text_layer(
        &mut self,
        language: Option<String>,
    ) -> PyResult<(usize, usize, usize)> {
        gate(self.doc.require_modify())?;
        let report = pdf_engine_core::add_searchable_text_layer(&mut self.doc, language.as_deref())
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to add searchable text: {}", e))
            })?;
        Ok((
            report.pages_seen,
            report.pages_recognized,
            report.words_inserted,
        ))
    }

    /// Rewrites the document as PDF/A-1b (`1b`) or PDF/A-2b (`2b`).
    ///
    /// The document changes only when the rewritten copy has no remaining
    /// structural issues. This is the engine's own check. It is not a
    /// veraPDF certificate, an Acrobat preflight, or a court acceptance.
    pub fn to_pdfa(&mut self, part: &str) -> PyResult<()> {
        gate(self.doc.require_modify())?;
        let Some(level) = pdf_engine_core::PdfALevel::parse(part) else {
            return Err(PyRuntimeError::new_err(
                "Failed to archive: Archive part was rejected.",
            ));
        };
        pdf_engine_core::convert_to_pdfa(&mut self.doc, level)
            .map_err(|error| PyRuntimeError::new_err(format!("Failed to archive: {error}")))
    }

    /// Reports archive issues for `1b` or `2b` without rewriting the document.
    pub fn pdfa_issues(&mut self, part: &str) -> PyResult<Vec<String>> {
        let Some(level) = pdf_engine_core::PdfALevel::parse(part) else {
            return Err(PyRuntimeError::new_err(
                "Failed to archive: Archive part was rejected.",
            ));
        };
        pdf_engine_core::validate_pdfa(&mut self.doc, level)
            .map_err(|error| PyRuntimeError::new_err(format!("Failed to archive: {error}")))
    }

    /// Retrieves a mutable reference to a page by 1-based or 0-based index.
    pub fn get_page(&mut self, index: usize) -> PyResult<PyPage> {
        let text_extract_allowed = self.doc.allows_copy();
        let zero_idx = if index > 0 && index <= self.active_pages.len() {
            index - 1
        } else {
            index
        };

        if zero_idx < self.active_pages.len() {
            let page = &self.active_pages[zero_idx];
            Ok(PyPage {
                page_number: page.page_number,
                page_id: page.page_id,
                contents_id: page.contents_id,
                ast: page.ast.clone(),
                paragraphs: page.paragraphs.clone(),
                metrics: page.metrics.clone(),
                faces: page.faces.clone(),
                hidden_ocg_props: page.hidden_ocg_props.clone(),
                text_extract_allowed,
            })
        } else {
            Err(PyValueError::new_err(format!(
                "Page index {} out of range (total pages: {})",
                index,
                self.active_pages.len()
            )))
        }
    }

    /// Updates the active page state back into the document.
    pub fn update_page(&mut self, page: &PyPage) -> PyResult<()> {
        gate(self.doc.require_modify())?;
        let zero_idx = if page.page_number > 0 && page.page_number <= self.active_pages.len() {
            page.page_number - 1
        } else {
            0
        };

        if let Some(contents_id) = page.contents_id {
            let new_bytes = page.to_bytes();
            let mut dict = pdf_engine_core::cos::PdfDictionary::new();
            dict.insert("Length", new_bytes.len() as i64);

            let new_stream = PdfObject::Stream(PdfStream::new(dict, new_bytes));
            self.doc.objects.insert(contents_id, new_stream);
        }

        if zero_idx < self.active_pages.len() {
            let text_extract_allowed = self.doc.allows_copy();
            self.active_pages[zero_idx] = PyPage {
                page_number: page.page_number,
                page_id: page.page_id,
                contents_id: page.contents_id,
                ast: page.ast.clone(),
                paragraphs: page.paragraphs.clone(),
                metrics: page.metrics.clone(),
                faces: page.faces.clone(),
                hidden_ocg_props: page.hidden_ocg_props.clone(),
                text_extract_allowed,
            };
        }

        Ok(())
    }

    /// Replaces a paragraph and applies a standard-14 face, size, color, alignment, or leading.
    ///
    /// `page_index` follows `get_page`: a number inside `1..=page_count` is 1-based.
    /// A face change registers Helvetica, Times, or Courier. Size, color, alignment,
    /// and leading can stay on the font the paragraph already uses. Leading is points.
    /// This is not an embedded bold twin and not a list or superscript edit.
    #[pyo3(signature = (page_index, paragraph_id, new_text, font=None, size=None, bold=None, italic=None, color=None, alignment=None, leading=None))]
    pub fn edit_paragraph_styled<'py>(
        &mut self,
        py: Python<'py>,
        page_index: usize,
        paragraph_id: usize,
        new_text: &str,
        font: Option<String>,
        size: Option<f64>,
        bold: Option<bool>,
        italic: Option<bool>,
        color: Option<Vec<f64>>,
        alignment: Option<String>,
        leading: Option<f64>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_modify())?;
        let zero_idx = if page_index > 0 && page_index <= self.active_pages.len() {
            page_index - 1
        } else {
            page_index
        };
        if zero_idx >= self.active_pages.len() {
            return Err(PyValueError::new_err(format!(
                "Page index {} out of range (total pages: {})",
                page_index,
                self.active_pages.len()
            )));
        }
        let text_extract_allowed = self.doc.allows_copy();
        let mut page = PyPage {
            page_number: self.active_pages[zero_idx].page_number,
            page_id: self.active_pages[zero_idx].page_id,
            contents_id: self.active_pages[zero_idx].contents_id,
            ast: self.active_pages[zero_idx].ast.clone(),
            paragraphs: self.active_pages[zero_idx].paragraphs.clone(),
            metrics: self.active_pages[zero_idx].metrics.clone(),
            faces: self.active_pages[zero_idx].faces.clone(),
            hidden_ocg_props: self.active_pages[zero_idx].hidden_ocg_props.clone(),
            text_extract_allowed,
        };
        let target = page
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.id == paragraph_id)
            .cloned()
            .ok_or_else(|| {
                PyValueError::new_err(format!(
                    "Paragraph id {} not found on page",
                    paragraph_id
                ))
            })?;
        let first_span = target
            .lines
            .first()
            .and_then(|line| line.spans.first())
            .ok_or_else(|| {
                PyRuntimeError::new_err("Target paragraph contains no styled spans")
            })?;
        let resource = first_span.font_name.trim_start_matches('/').to_string();
        let current_size = first_span.font_size;
        let (css_family, weight, css_style) = match page.faces.get(&resource) {
            Some(face) => (face.family.clone(), face.weight, face.style.clone()),
            None => (String::new(), 400, "normal".to_string()),
        };
        let parsed_alignment = match alignment {
            Some(raw) => Some(ParagraphStyle::parse_alignment(&raw).map_err(map_engine_value)?),
            None => None,
        };
        let parsed_color = match color {
            Some(values) => {
                Some(ParagraphStyle::color_from_components(&values).map_err(map_engine_value)?)
            }
            None => None,
        };
        let style = ParagraphStyle {
            family: font,
            size,
            bold,
            italic,
            color: parsed_color,
            alignment: parsed_alignment,
            leading,
        };
        let prepared = style
            .prepare(
                &resource,
                current_size,
                target.leading,
                target.alignment,
                &css_family,
                weight,
                &css_style,
            )
            .map_err(map_engine_value)?;
        let metrics = if prepared.registers_font {
            page.metrics.clone()
        } else if let Some(face) = page.faces.get(&prepared.font_resource) {
            face.metrics.clone()
        } else {
            page.metrics.clone()
        };
        SurgicalEditor::edit_paragraph_prepared(
            &mut page.ast,
            &target,
            new_text,
            &metrics,
            &prepared,
        )
        .map_err(map_engine_value)?;
        if prepared.registers_font {
            page.faces.insert(
                prepared.font_resource.clone(),
                ResolvedFont {
                    metrics: FontMetrics::new(0, 255, vec![500.0; 256], 500.0),
                    cmap: None,
                    family: prepared.css_family.clone(),
                    weight: prepared.weight,
                    style: prepared.css_style.clone(),
                    is_composite: false,
                },
            );
        }
        page.paragraphs = reconstruct_layout(
            &page.ast,
            &page.metrics,
            &page.faces,
            &page.hidden_ocg_props,
        )?;
        if prepared.registers_font {
            if let Some(base_font) = prepared.base_font.as_deref() {
                pdf_engine_core::watermark::helpers::ensure_font_resource(
                    &mut self.doc,
                    page.page_id,
                    &prepared.font_resource,
                    base_font,
                )
                .map_err(map_engine_value)?;
            }
        }
        self.update_page(&page)?;
        let alignment_name = match prepared.alignment {
            TextAlignment::Left => "left",
            TextAlignment::Center => "center",
            TextAlignment::Right => "right",
            TextAlignment::Justified => "justified",
        };
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("font_resource", &prepared.font_resource)?;
        dict.set_item("base_font", prepared.base_font.clone())?;
        dict.set_item("font_size", prepared.size)?;
        dict.set_item("alignment", alignment_name)?;
        dict.set_item("leading", prepared.leading)?;
        Ok(dict)
    }

    /// Appends content-stream text inside a rectangle and rebuilds the page handles.
    ///
    /// `page_index` follows `get_page`: a number inside `1..=page_count` is 1-based.
    /// Omitted style fields become Helvetica, 12 points, black, left, and a leading
    /// of 1.2 times the size. The text is not a `/FreeText` annotation. Lines whose
    /// baseline falls below the rectangle are omitted.
    #[pyo3(signature = (page_index, x, y, width, height, text, font=None, size=None, bold=None, italic=None, color=None, alignment=None, leading=None))]
    pub fn add_text_box<'py>(
        &mut self,
        py: Python<'py>,
        page_index: usize,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        text: &str,
        font: Option<String>,
        size: Option<f64>,
        bold: Option<bool>,
        italic: Option<bool>,
        color: Option<Vec<f64>>,
        alignment: Option<String>,
        leading: Option<f64>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_modify())?;
        let parsed_alignment = match alignment {
            Some(raw) => Some(ParagraphStyle::parse_alignment(&raw).map_err(map_engine_value)?),
            None => None,
        };
        let parsed_color = match color {
            Some(values) => {
                Some(ParagraphStyle::color_from_components(&values).map_err(map_engine_value)?)
            }
            None => None,
        };
        let spec = TextBoxSpec {
            text: text.to_string(),
            x,
            y,
            width,
            height,
            style: ParagraphStyle {
                family: font,
                size,
                bold,
                italic,
                color: parsed_color,
                alignment: parsed_alignment,
                leading,
            },
        };
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let appended = pdf_engine_core::append_text_box(&mut self.doc, zero_idx, &spec)
            .map_err(map_engine_value)?;
        self.refresh_handles()?;
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("font_resource", &appended.font_resource)?;
        dict.set_item("base_font", &appended.base_font)?;
        dict.set_item("line_count", appended.line_count)?;
        Ok(dict)
    }

    /// Appends a typed, drawn, or image signature or initials as page content.
    ///
    /// `page_index` follows `get_page`: a number inside `1..=page_count` is 1-based.
    /// The mark is not an `/Ink` annotation, not an AcroForm field, and not PKCS#7.
    /// The returned dict does not copy the typed text or the image bytes.
    #[pyo3(signature = (page_index, kind, mode, x, y, width, height, text=None, strokes=None, image=None))]
    pub fn add_fill_sign<'py>(
        &mut self,
        py: Python<'py>,
        page_index: usize,
        kind: &str,
        mode: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        text: Option<String>,
        strokes: Option<Vec<Vec<Vec<f64>>>>,
        image: Option<Vec<u8>>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_modify())?;
        let parsed_kind = match kind {
            "signature" => FillSignKind::Signature,
            "initials" => FillSignKind::Initials,
            _ => return Err(fill_sign_input("The kind is signature or initials.")),
        };
        let mark = if mode == "typed" {
            let Some(typed) = text else {
                return Err(fill_sign_input("The mark does not match the mode."));
            };
            if strokes.is_some() || image.is_some() {
                return Err(fill_sign_input("The mark does not match the mode."));
            }
            FillSignMark::Typed(typed)
        } else if mode == "drawn" {
            let Some(raw_strokes) = strokes else {
                return Err(fill_sign_input("The mark does not match the mode."));
            };
            if text.is_some() || image.is_some() {
                return Err(fill_sign_input("The mark does not match the mode."));
            }
            let mut parsed = Vec::with_capacity(raw_strokes.len());
            for stroke in raw_strokes {
                let mut points = Vec::with_capacity(stroke.len());
                for point in stroke {
                    if point.len() != 2 {
                        return Err(fill_sign_input("The mark does not match the mode."));
                    }
                    points.push((point[0], point[1]));
                }
                parsed.push(points);
            }
            FillSignMark::Drawn(parsed)
        } else if mode == "image" {
            let Some(bytes) = image else {
                return Err(fill_sign_input("The mark does not match the mode."));
            };
            if text.is_some() || strokes.is_some() {
                return Err(fill_sign_input("The mark does not match the mode."));
            }
            FillSignMark::Image(bytes)
        } else {
            return Err(fill_sign_input("The mode is typed, drawn, or image."));
        };
        let spec = FillSignSpec {
            kind: parsed_kind,
            x,
            y,
            width,
            height,
            mark,
        };
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let appended = pdf_engine_core::append_fill_sign(&mut self.doc, zero_idx, &spec)
            .map_err(map_engine_value)?;
        self.refresh_handles()?;
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("kind", &appended.kind)?;
        dict.set_item("mode", &appended.mode)?;
        dict.set_item("resource", &appended.resource)?;
        Ok(dict)
    }

    /// Extracts embedded font binaries (TrueType / OpenType) from a specific page.
    /// Returns a dict mapping font names (e.g. "F1") to raw font file bytes.
    pub fn get_page_fonts(
        &mut self,
        index: usize,
    ) -> PyResult<std::collections::HashMap<String, Vec<u8>>> {
        let zero_idx = if index > 0 && index <= self.page_ids.len() {
            index - 1
        } else {
            index
        };

        if zero_idx < self.page_ids.len() {
            let page_id = self.page_ids[zero_idx];
            self.doc
                .extract_page_fonts(page_id)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract fonts: {}", e)))
        } else {
            Err(PyValueError::new_err(format!(
                "Page index {} out of range",
                index
            )))
        }
    }

    /// Extracts all Image XObjects on a given page (1-indexed or 0-indexed).
    pub fn get_page_images(&mut self, index: usize) -> PyResult<Vec<PyImageInfo>> {
        let zero_idx = if index > 0 && index <= self.page_ids.len() {
            index - 1
        } else {
            index
        };

        if zero_idx < self.page_ids.len() {
            let page_id = self.page_ids[zero_idx];
            let images = pdf_engine_core::images::extract_page_images(&mut self.doc, page_id)
                .map_err(|e| {
                    PyRuntimeError::new_err(format!("Failed to extract page images: {}", e))
                })?;

            Ok(images
                .into_iter()
                .map(|img| PyImageInfo {
                    id: img.object_id.number,
                    name: img.resource_name,
                    width_px: img.width_px,
                    height_px: img.height_px,
                    color_space: img.color_space,
                    bits_per_component: img.bits_per_component,
                    filter: img.filter,
                    byte_size: img.byte_size,
                    min_x: img.bbox.min_x,
                    min_y: img.bbox.min_y,
                    max_x: img.bbox.max_x,
                    max_y: img.bbox.max_y,
                })
                .collect())
        } else {
            Err(PyValueError::new_err(format!(
                "Page index {} out of range",
                index
            )))
        }
    }

    /// Retrieves raw image file bytes and MIME type for a given image object ID.
    pub fn get_image_binary(&mut self, object_id_num: u32) -> PyResult<(Vec<u8>, String)> {
        gate(self.doc.require_copy())?;
        let obj_id = ObjectId::new(object_id_num);
        let (bytes, mime) = pdf_engine_core::images::get_image_binary(&mut self.doc, obj_id)
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to retrieve image binary: {}", e))
            })?;
        Ok((bytes, mime.to_string()))
    }

    /// Surgically replaces an existing Image XObject in the document with new JPEG or PNG bytes.
    pub fn replace_image(&mut self, object_id_num: u32, new_bytes: &[u8]) -> PyResult<()> {
        gate(self.doc.require_modify())?;
        let obj_id = ObjectId::new(object_id_num);
        pdf_engine_core::images::replace_image_content(&mut self.doc, obj_id, new_bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to replace image: {}", e)))
    }

    /// Extracts all interactive AcroForm fields from the document.
    pub fn get_form_fields(&mut self) -> PyResult<Vec<PyFormField>> {
        let fields =
            pdf_engine_core::forms::extract_document_forms(&mut self.doc).map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to extract form fields: {}", e))
            })?;

        Ok(fields
            .into_iter()
            .map(|f| PyFormField {
                id: f.id.number,
                name: f.name,
                alt_name: f.alt_name,
                field_type: f.field_type.as_str().to_string(),
                value: f.value,
                default_value: f.default_value,
                min_x: f.rect.min_x,
                min_y: f.rect.min_y,
                max_x: f.rect.max_x,
                max_y: f.rect.max_y,
                page_number: f.page_number,
                options: f.options,
                is_read_only: f.is_read_only,
                is_required: f.is_required,
                is_multiline: f.is_multiline,
                max_length: f.max_length,
            })
            .collect())
    }

    /// Creates and places a new interactive form field on a specific page.
    #[pyo3(signature = (
        page_number,
        name,
        field_type,
        min_x,
        min_y,
        max_x,
        max_y,
        value = None,
        default_value = None,
        alt_name = None,
        options = None,
        is_read_only = false,
        is_required = false,
        is_multiline = false,
        max_length = None,
        font_size = None
    ))]
    pub fn add_form_field(
        &mut self,
        page_number: usize,
        name: String,
        field_type: String,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        value: Option<String>,
        default_value: Option<String>,
        alt_name: Option<String>,
        options: Option<Vec<String>>,
        is_read_only: bool,
        is_required: bool,
        is_multiline: bool,
        max_length: Option<usize>,
        font_size: Option<f64>,
    ) -> PyResult<PyFormField> {
        gate(self.doc.require_form_create())?;
        let f_type = pdf_engine_core::forms::FormFieldType::from_str(&field_type);
        let opts = pdf_engine_core::forms::FormFieldCreateOptions {
            name,
            field_type: f_type,
            rect: pdf_engine_core::layout::geometry::Rect::new(min_x, min_y, max_x, max_y),
            value,
            default_value,
            alt_name,
            options,
            is_read_only,
            is_required,
            is_multiline,
            max_length,
            font_size,
        };

        let created = pdf_engine_core::forms::create_form_field(&mut self.doc, page_number, &opts)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create form field: {}", e)))?;

        Ok(PyFormField {
            id: created.id.number,
            name: created.name,
            alt_name: created.alt_name,
            field_type: created.field_type.as_str().to_string(),
            value: created.value,
            default_value: created.default_value,
            min_x: created.rect.min_x,
            min_y: created.rect.min_y,
            max_x: created.rect.max_x,
            max_y: created.rect.max_y,
            page_number: created.page_number,
            options: created.options,
            is_read_only: created.is_read_only,
            is_required: created.is_required,
            is_multiline: created.is_multiline,
            max_length: created.max_length,
        })
    }

    /// Deletes an existing interactive form field from the document by name.
    pub fn delete_form_field(&mut self, name: &str) -> PyResult<bool> {
        gate(self.doc.require_form_create())?;
        pdf_engine_core::forms::delete_form_field(&mut self.doc, name)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete form field: {}", e)))
    }

    /// Updates bounding box or properties of an existing form field.
    #[pyo3(signature = (
        name,
        min_x = None,
        min_y = None,
        max_x = None,
        max_y = None,
        alt_name = None,
        is_read_only = None,
        is_required = None,
        is_multiline = None
    ))]
    pub fn update_form_field(
        &mut self,
        name: &str,
        min_x: Option<f64>,
        min_y: Option<f64>,
        max_x: Option<f64>,
        max_y: Option<f64>,
        alt_name: Option<String>,
        is_read_only: Option<bool>,
        is_required: Option<bool>,
        is_multiline: Option<bool>,
    ) -> PyResult<Option<PyFormField>> {
        gate(self.doc.require_form_create())?;
        let rect = match (min_x, min_y, max_x, max_y) {
            (Some(x1), Some(y1), Some(x2), Some(y2)) => {
                Some(pdf_engine_core::layout::geometry::Rect::new(x1, y1, x2, y2))
            }
            _ => None,
        };

        let opts = pdf_engine_core::forms::FormFieldUpdateOptions {
            rect,
            alt_name,
            is_read_only,
            is_required,
            is_multiline,
        };

        let updated = pdf_engine_core::forms::update_form_field(&mut self.doc, name, &opts)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to update form field: {}", e)))?;

        Ok(updated.map(|f| PyFormField {
            id: f.id.number,
            name: f.name,
            alt_name: f.alt_name,
            field_type: f.field_type.as_str().to_string(),
            value: f.value,
            default_value: f.default_value,
            min_x: f.rect.min_x,
            min_y: f.rect.min_y,
            max_x: f.rect.max_x,
            max_y: f.rect.max_y,
            page_number: f.page_number,
            options: f.options,
            is_read_only: f.is_read_only,
            is_required: f.is_required,
            is_multiline: f.is_multiline,
            max_length: f.max_length,
        }))
    }

    /// Fills a single form field by name or ID.
    pub fn fill_form_field(&mut self, name_or_id: &str, value: &str) -> PyResult<bool> {
        gate(self.doc.require_fill())?;
        pdf_engine_core::forms::fill_field_value(&mut self.doc, name_or_id, value)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to fill form field: {}", e)))
    }

    /// Fills multiple form fields in a single batch pass.
    pub fn fill_form_fields(
        &mut self,
        values: std::collections::HashMap<String, String>,
    ) -> PyResult<usize> {
        gate(self.doc.require_fill())?;
        pdf_engine_core::forms::fill_fields_batch(&mut self.doc, &values).map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to batch fill form fields: {}", e))
        })
    }

    /// Flattens all interactive form fields into permanent page content and strips widget annotations.
    pub fn flatten_forms(&mut self) -> PyResult<usize> {
        gate(self.doc.require_modify())?;
        let count = pdf_engine_core::forms::flatten_document_forms(&mut self.doc).map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to flatten form fields: {}", e))
        })?;

        self.reload_active_pages()?;
        Ok(count)
    }

    /// Detects smart form field candidates from vector drawings and layout labels on a page (1-indexed).
    pub fn detect_form_fields(
        &mut self,
        page_number: usize,
    ) -> PyResult<Vec<PyDetectedFormFieldCandidate>> {
        let candidates =
            pdf_engine_core::forms::detect_form_field_candidates(&mut self.doc, page_number)
                .map_err(|e| {
                    PyRuntimeError::new_err(format!("Form field detection failed: {}", e))
                })?;
        Ok(candidates
            .into_iter()
            .map(PyDetectedFormFieldCandidate::from_core)
            .collect())
    }

    /// Converts detected form field candidates into interactive AcroForm widgets on a page (1-indexed).
    /// If `candidates` is None, detects candidates automatically and applies them.
    #[pyo3(signature = (page_number, candidates=None))]
    pub fn apply_detected_form_fields(
        &mut self,
        page_number: usize,
        candidates: Option<Vec<PyDetectedFormFieldCandidate>>,
    ) -> PyResult<Vec<PyFormField>> {
        gate(self.doc.require_form_create())?;
        let core_candidates = match candidates {
            Some(cands) => cands.into_iter().map(|c| c.to_core()).collect(),
            None => {
                pdf_engine_core::forms::detect_form_field_candidates(&mut self.doc, page_number)
                    .map_err(|e| {
                        PyRuntimeError::new_err(format!("Form field detection failed: {}", e))
                    })?
            }
        };

        let created = pdf_engine_core::forms::apply_detected_form_fields(
            &mut self.doc,
            page_number,
            &core_candidates,
        )
        .map_err(|e| {
            PyRuntimeError::new_err(format!("Applying detected form fields failed: {}", e))
        })?;

        self.reload_active_pages()?;

        Ok(created
            .into_iter()
            .map(|f| PyFormField {
                id: f.id.number,
                name: f.name,
                alt_name: f.alt_name,
                field_type: f.field_type.as_str().to_string(),
                value: f.value,
                default_value: f.default_value,
                min_x: f.rect.min_x,
                min_y: f.rect.min_y,
                max_x: f.rect.max_x,
                max_y: f.rect.max_y,
                page_number: f.page_number,
                options: f.options,
                is_read_only: f.is_read_only,
                is_required: f.is_required,
                is_multiline: f.is_multiline,
                max_length: f.max_length,
            })
            .collect())
    }

    /// Extracts all non-widget annotations from a specific page.
    pub fn get_page_annotations(&mut self, page_index: usize) -> PyResult<Vec<PyAnnotation>> {
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let annots = pdf_engine_core::annots::extract_page_annotations(&mut self.doc, zero_idx)
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to extract page annotations: {}", e))
            })?;

        Ok(annots.into_iter().map(PyAnnotation::from_core).collect())
    }

    /// Extracts all non-widget annotations across the entire document.
    pub fn get_all_annotations(&mut self) -> PyResult<Vec<PyAnnotation>> {
        let annots =
            pdf_engine_core::annots::extract_all_annotations(&mut self.doc).map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to extract document annotations: {}", e))
            })?;

        Ok(annots.into_iter().map(PyAnnotation::from_core).collect())
    }

    /// Adds a text markup annotation (Highlight, Underline, StrikeOut) to a page.
    #[pyo3(signature = (page_index, subtype, min_x, min_y, max_x, max_y, color=None, opacity=None, contents=None))]
    pub fn add_text_markup(
        &mut self,
        page_index: usize,
        subtype: &str,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        color: Option<Vec<f64>>,
        opacity: Option<f64>,
        contents: Option<String>,
    ) -> PyResult<u32> {
        gate(self.doc.require_annotate())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let st = AnnotationSubtype::from_pdf_name(subtype);
        let rect = Rect::new(min_x, min_y, max_x, max_y);
        let rgb = color.and_then(|c| {
            if c.len() >= 3 {
                Some([c[0], c[1], c[2]])
            } else {
                None
            }
        });
        let annot_id = pdf_engine_core::annots::add_text_markup(
            &mut self.doc,
            zero_idx,
            st,
            rect,
            None,
            rgb,
            opacity,
            contents.as_deref(),
        )
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to add text markup: {}", e)))?;

        Ok(annot_id.number)
    }

    /// Adds an interactive clickable URI link annotation to a page.
    #[pyo3(signature = (page_index, min_x, min_y, max_x, max_y, uri, show_border=None))]
    pub fn add_link_uri(
        &mut self,
        page_index: usize,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        uri: &str,
        show_border: Option<bool>,
    ) -> PyResult<u32> {
        gate(self.doc.require_annotate())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let rect = Rect::new(min_x, min_y, max_x, max_y);
        let annot_id = pdf_engine_core::annots::add_link_uri(
            &mut self.doc,
            zero_idx,
            rect,
            uri,
            show_border.unwrap_or(false),
        )
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to add link: {}", e)))?;

        Ok(annot_id.number)
    }

    /// Adds an internal document jump link annotation targeting another page.
    pub fn add_link_goto(
        &mut self,
        page_index: usize,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        target_page_index: usize,
    ) -> PyResult<u32> {
        gate(self.doc.require_annotate())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let target_zero = if target_page_index > 0 && target_page_index <= self.page_ids.len() {
            target_page_index - 1
        } else {
            target_page_index
        };
        let rect = Rect::new(min_x, min_y, max_x, max_y);
        let annot_id =
            pdf_engine_core::annots::add_link_goto(&mut self.doc, zero_idx, rect, target_zero)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to add goto link: {}", e)))?;

        Ok(annot_id.number)
    }

    /// Adds a vector rubber stamp annotation to a page.
    #[pyo3(signature = (page_index, stamp_type, min_x=None, min_y=None, max_x=None, max_y=None, custom_text=None, color=None, date_str=None))]
    pub fn add_stamp(
        &mut self,
        page_index: usize,
        stamp_type: &str,
        min_x: Option<f64>,
        min_y: Option<f64>,
        max_x: Option<f64>,
        max_y: Option<f64>,
        custom_text: Option<&str>,
        color: Option<Vec<f64>>,
        date_str: Option<&str>,
    ) -> PyResult<u32> {
        gate(self.doc.require_annotate())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let st = StampType::from_name_or_text(stamp_type);
        let rect = match (min_x, min_y, max_x, max_y) {
            (Some(x1), Some(y1), Some(x2), Some(y2)) => Some(Rect::new(x1, y1, x2, y2)),
            _ => None,
        };
        let rgb = color.and_then(|c| {
            if c.len() >= 3 {
                Some([c[0], c[1], c[2]])
            } else {
                None
            }
        });
        let annot_id = pdf_engine_core::annots::add_stamp(
            &mut self.doc,
            zero_idx,
            st,
            rect,
            custom_text,
            rgb,
            date_str,
        )
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to add stamp: {}", e)))?;

        Ok(annot_id.number)
    }

    /// Adds an ink stroke or a vector shape. `Arrow` is a line with an open arrow.
    #[pyo3(signature = (page_index, kind, points, stroke, fill=None, line_width=1.5, opacity=1.0))]
    pub fn add_shape(
        &mut self,
        page_index: usize,
        kind: &str,
        points: Vec<Vec<f64>>,
        stroke: Vec<f64>,
        fill: Option<Vec<f64>>,
        line_width: f64,
        opacity: f64,
    ) -> PyResult<u32> {
        gate(self.doc.require_annotate())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let (shape_kind, ending) = match kind {
            "Arrow" => (ShapeKind::Line, Some("OpenArrow")),
            other => (
                ShapeKind::from_name(other).ok_or_else(|| {
                    PyRuntimeError::new_err("Failed to add shape: Shape kind was rejected.")
                })?,
                None,
            ),
        };
        let stroke = if stroke.len() >= 3 {
            [stroke[0], stroke[1], stroke[2]]
        } else {
            return Err(PyRuntimeError::new_err(
                "Failed to add shape: Shape color was rejected.",
            ));
        };
        let fill = match fill {
            Some(channels) if channels.len() >= 3 => Some([channels[0], channels[1], channels[2]]),
            Some(_) => {
                return Err(PyRuntimeError::new_err(
                    "Failed to add shape: Shape color was rejected.",
                ));
            }
            None => None,
        };
        let mut stored = Vec::with_capacity(points.len());
        for point in &points {
            if point.len() < 2 || !point[0].is_finite() || !point[1].is_finite() {
                return Err(PyRuntimeError::new_err(
                    "Failed to add shape: Shape point was rejected.",
                ));
            }
            stored.push([point[0], point[1]]);
        }
        let annot_id = pdf_engine_core::annots::add_shape(
            &mut self.doc,
            zero_idx,
            shape_kind,
            &stored,
            ShapeStyle {
                stroke,
                fill,
                line_width,
                opacity,
                line_ending: ending,
            },
        )
        .map_err(|error| PyRuntimeError::new_err(format!("Failed to add shape: {error}")))?;
        Ok(annot_id.number)
    }

    /// Deletes an annotation from a page.
    pub fn delete_annotation(&mut self, page_index: usize, annot_id: u32) -> PyResult<bool> {
        gate(self.doc.require_annotate())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        pdf_engine_core::annots::delete_annotation(&mut self.doc, zero_idx, ObjectId::new(annot_id))
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete annotation: {}", e)))
    }

    /// Flattens all visual annotations (highlights, underlines, strikeouts, stamps) into permanent page graphics.
    #[pyo3(signature = (page_index=None))]
    pub fn flatten_annotations(&mut self, page_index: Option<usize>) -> PyResult<usize> {
        gate(self.doc.require_flatten_annotations())?;
        let target_idx = page_index.map(|idx| {
            if idx > 0 && idx <= self.page_ids.len() {
                idx - 1
            } else {
                idx
            }
        });
        let count = pdf_engine_core::annots::flatten_annotations(&mut self.doc, target_idx)
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to flatten annotations: {}", e))
            })?;

        self.reload_active_pages()?;
        Ok(count)
    }

    /// Rotates a page (0-based or 1-based index) by degrees (normalized to 0, 90, 180, 270).
    pub fn rotate_page(&mut self, page_index: usize, degrees: i32) -> PyResult<i32> {
        gate(self.doc.require_assemble())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        pdf_engine_core::ops::rotate_page(&mut self.doc, zero_idx, degrees)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to rotate page: {}", e)))
    }

    /// Gets the current rotation of a page in degrees.
    pub fn get_page_rotation(&mut self, page_index: usize) -> PyResult<i32> {
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        pdf_engine_core::ops::get_page_rotation(&mut self.doc, zero_idx)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to get page rotation: {}", e)))
    }

    /// Rotates all pages in the document by `degrees`.
    pub fn rotate_all_pages(&mut self, degrees: i32) -> PyResult<()> {
        gate(self.doc.require_assemble())?;
        pdf_engine_core::ops::rotate_all_pages(&mut self.doc, degrees)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to rotate all pages: {}", e)))
    }

    /// Extracts specified pages into a new self-contained PyPdfDocument.
    pub fn extract_pages(&mut self, page_indices: Vec<usize>) -> PyResult<Self> {
        gate(self.doc.require_copy())?;
        let zero_indices: Vec<usize> = page_indices
            .into_iter()
            .map(|idx| {
                if idx > 0 && idx <= self.page_ids.len() {
                    idx - 1
                } else {
                    idx
                }
            })
            .collect();
        let extracted = pdf_engine_core::ops::extract_pages(&mut self.doc, &zero_indices)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract pages: {}", e)))?;
        Self::from_doc(extracted)
    }

    /// Deletes specified pages from the document.
    pub fn delete_pages(&mut self, page_indices: Vec<usize>) -> PyResult<()> {
        gate(self.doc.require_assemble())?;
        let zero_indices: Vec<usize> = page_indices
            .into_iter()
            .map(|idx| {
                if idx > 0 && idx <= self.page_ids.len() {
                    idx - 1
                } else {
                    idx
                }
            })
            .collect();
        pdf_engine_core::ops::delete_pages(&mut self.doc, &zero_indices)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete pages: {}", e)))?;
        let updated = Self::from_doc(self.doc.clone())?;
        self.doc = updated.doc;
        self.page_ids = updated.page_ids;
        self.active_pages = updated.active_pages;
        Ok(())
    }

    /// Reorders the document pages according to a 0-based permutation.
    pub fn reorder_pages(&mut self, new_order: Vec<usize>) -> PyResult<()> {
        gate(self.doc.require_assemble())?;
        pdf_engine_core::ops::reorder_pages(&mut self.doc, &new_order)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to reorder pages: {}", e)))?;
        let updated = Self::from_doc(self.doc.clone())?;
        self.doc = updated.doc;
        self.page_ids = updated.page_ids;
        self.active_pages = updated.active_pages;
        Ok(())
    }

    /// Merges another document's pages into this document.
    pub fn merge_with(&mut self, other: &PyPdfDocument) -> PyResult<()> {
        gate(self.doc.require_assemble())?;
        gate(other.doc.require_copy())?;
        let mut dest = self.doc.clone();
        let mut source = other.doc.clone();
        let merged = pdf_engine_core::ops::append_document(&mut dest, &mut source)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to merge: {}", e)))?;
        let updated = Self::from_doc(merged)?;
        self.doc = updated.doc;
        self.page_ids = updated.page_ids;
        self.active_pages = updated.active_pages;
        Ok(())
    }

    /// Injects dynamic headers, footers, or page numbers (e.g. "Página {page} de {total}").
    #[pyo3(signature = (format=None, position=None, font_size=None, color=None, margin=None, start_page_num=None, skip_first_page=None, page_indices=None))]
    pub fn add_pagination(
        &mut self,
        format: Option<String>,
        position: Option<String>,
        font_size: Option<f64>,
        color: Option<(f64, f64, f64)>,
        margin: Option<f64>,
        start_page_num: Option<usize>,
        skip_first_page: Option<bool>,
        page_indices: Option<Vec<usize>>,
    ) -> PyResult<usize> {
        gate(self.doc.require_modify())?;
        let pos = match position.as_deref() {
            Some(s) => PaginationPosition::parse(s).ok_or_else(|| {
                PyValueError::new_err(format!("Invalid pagination position: '{}'", s))
            })?,
            None => PaginationPosition::BottomCenter,
        };

        let config = PaginationConfig {
            format: format.unwrap_or_else(|| "Página {page} de {total}".to_string()),
            position: pos,
            font_size: font_size.unwrap_or(9.0),
            color: color
                .map(|(r, g, b)| [r, g, b])
                .unwrap_or([0.35, 0.35, 0.35]),
            margin: margin.unwrap_or(36.0),
            start_page_num: start_page_num.unwrap_or(1),
            skip_first_page: skip_first_page.unwrap_or(false),
            page_indices,
        };

        let count = pdf_engine_core::watermark::apply_pagination(&mut self.doc, &config)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to apply pagination: {}", e)))?;

        let updated = Self::from_doc(self.doc.clone())?;
        self.doc = updated.doc;
        self.page_ids = updated.page_ids;
        self.active_pages = updated.active_pages;

        Ok(count)
    }

    /// Adds a semi-transparent text watermark to target pages.
    #[pyo3(signature = (text, font_size=None, color=None, opacity=None, rotation_degrees=None, placement=None, page_indices=None))]
    pub fn add_text_watermark(
        &mut self,
        text: String,
        font_size: Option<f64>,
        color: Option<(f64, f64, f64)>,
        opacity: Option<f64>,
        rotation_degrees: Option<f64>,
        placement: Option<String>,
        page_indices: Option<Vec<usize>>,
    ) -> PyResult<usize> {
        gate(self.doc.require_modify())?;
        let place = match placement.as_deref() {
            Some(s) => WatermarkPlacement::parse(s).ok_or_else(|| {
                PyValueError::new_err(format!("Invalid watermark placement: '{}'", s))
            })?,
            None => WatermarkPlacement::Background,
        };

        let config = TextWatermarkConfig {
            text,
            font_size: font_size.unwrap_or(52.0),
            color: color
                .map(|(r, g, b)| [r, g, b])
                .unwrap_or([0.80, 0.20, 0.20]),
            opacity: opacity.unwrap_or(0.22),
            rotation_degrees: rotation_degrees.unwrap_or(45.0),
            placement: place,
            page_indices,
        };

        let count = pdf_engine_core::watermark::apply_text_watermark(&mut self.doc, &config)
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to apply text watermark: {}", e))
            })?;

        let updated = Self::from_doc(self.doc.clone())?;
        self.doc = updated.doc;
        self.page_ids = updated.page_ids;
        self.active_pages = updated.active_pages;

        Ok(count)
    }

    /// Adds a semi-transparent image watermark to target pages from raw JPEG or PNG bytes.
    #[pyo3(signature = (image_bytes, width=None, height=None, opacity=None, rotation_degrees=None, placement=None, page_indices=None))]
    pub fn add_image_watermark(
        &mut self,
        image_bytes: Vec<u8>,
        width: Option<f64>,
        height: Option<f64>,
        opacity: Option<f64>,
        rotation_degrees: Option<f64>,
        placement: Option<String>,
        page_indices: Option<Vec<usize>>,
    ) -> PyResult<usize> {
        gate(self.doc.require_modify())?;
        let place = match placement.as_deref() {
            Some(s) => WatermarkPlacement::parse(s).ok_or_else(|| {
                PyValueError::new_err(format!("Invalid watermark placement: '{}'", s))
            })?,
            None => WatermarkPlacement::Background,
        };

        let config = ImageWatermarkConfig {
            image_bytes,
            width,
            height,
            opacity: opacity.unwrap_or(0.25),
            rotation_degrees: rotation_degrees.unwrap_or(0.0),
            placement: place,
            page_indices,
        };

        let count = pdf_engine_core::watermark::apply_image_watermark(&mut self.doc, &config)
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to apply image watermark: {}", e))
            })?;

        let updated = Self::from_doc(self.doc.clone())?;
        self.doc = updated.doc;
        self.page_ids = updated.page_ids;
        self.active_pages = updated.active_pages;

        Ok(count)
    }

    /// Redacts specific rectangular bounding boxes on a target page.
    #[pyo3(signature = (page_index, regions, fill_color=None, overlay_text=None, text_color=None, font_size=None, prune_annotations=None))]
    pub fn redact_regions(
        &mut self,
        page_index: usize,
        regions: Vec<(f64, f64, f64, f64)>,
        fill_color: Option<(f64, f64, f64)>,
        overlay_text: Option<String>,
        text_color: Option<(f64, f64, f64)>,
        font_size: Option<f64>,
        prune_annotations: Option<bool>,
    ) -> PyResult<PyRedactionSummary> {
        gate(self.doc.require_modify())?;
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };

        let mut config = RedactionConfig::default();
        if let Some((r, g, b)) = fill_color {
            config.fill_color = [r, g, b];
        }
        if let Some(text) = overlay_text {
            config.overlay_text = Some(text);
        }
        if let Some((r, g, b)) = text_color {
            config.text_color = [r, g, b];
        }
        if let Some(sz) = font_size {
            config.font_size = Some(sz);
        }
        if let Some(prune) = prune_annotations {
            config.prune_annotations = prune;
        }

        let rects: Vec<Rect> = regions
            .into_iter()
            .map(|(x1, y1, x2, y2)| Rect::new(x1, y1, x2, y2))
            .collect();

        let summary = pdf_engine_core::redact::redact_document_rectangles(
            &mut self.doc,
            zero_idx,
            &rects,
            &config,
        )
        .map_err(|e| PyRuntimeError::new_err(format!("Redaction failed: {}", e)))?;

        self.reload_active_pages()?;
        Ok(PyRedactionSummary::from_core(summary))
    }

    /// Scans the document for a sensitive data pattern and redacts all occurrences.
    #[pyo3(signature = (pattern_type, custom_query=None, case_sensitive=None, page_indices=None, fill_color=None, overlay_text=None, text_color=None, font_size=None, prune_annotations=None, scrub_metadata=None, padding=None))]
    pub fn redact_pattern(
        &mut self,
        pattern_type: String,
        custom_query: Option<String>,
        case_sensitive: Option<bool>,
        page_indices: Option<Vec<usize>>,
        fill_color: Option<(f64, f64, f64)>,
        overlay_text: Option<String>,
        text_color: Option<(f64, f64, f64)>,
        font_size: Option<f64>,
        prune_annotations: Option<bool>,
        scrub_metadata: Option<bool>,
        padding: Option<f64>,
    ) -> PyResult<Vec<PyRedactionSummary>> {
        gate(self.doc.require_modify())?;
        let pattern = RedactionPattern::from_name(
            &pattern_type,
            custom_query.as_deref(),
            case_sensitive.unwrap_or(false),
        )
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "Invalid redaction pattern type: '{}'. Valid options: email, phone, ssn, credit_card, rfc, curp, text",
                pattern_type
            ))
        })?;

        let mut config = RedactionConfig::default();
        if let Some((r, g, b)) = fill_color {
            config.fill_color = [r, g, b];
        }
        if let Some(text) = overlay_text {
            config.overlay_text = Some(text);
        }
        if let Some((r, g, b)) = text_color {
            config.text_color = [r, g, b];
        }
        if let Some(sz) = font_size {
            config.font_size = Some(sz);
        }
        if let Some(prune) = prune_annotations {
            config.prune_annotations = prune;
        }
        if let Some(scrub) = scrub_metadata {
            config.scrub_metadata = scrub;
        }
        if let Some(pad) = padding {
            config.padding = pad;
        }

        let zero_indices: Option<Vec<usize>> = page_indices.map(|indices| {
            indices
                .into_iter()
                .map(|idx| {
                    if idx > 0 && idx <= self.page_ids.len() {
                        idx - 1
                    } else {
                        idx
                    }
                })
                .collect()
        });

        let summaries = pdf_engine_core::redact::redact_document_pattern(
            &mut self.doc,
            &pattern,
            zero_indices.as_deref(),
            &config,
        )
        .map_err(|e| PyRuntimeError::new_err(format!("Pattern redaction failed: {}", e)))?;

        self.reload_active_pages()?;
        Ok(summaries
            .into_iter()
            .map(PyRedactionSummary::from_core)
            .collect())
    }

    /// Redacts all occurrences of a search text string across target pages.
    #[pyo3(signature = (query, case_sensitive=None, page_indices=None, fill_color=None, overlay_text=None, text_color=None, font_size=None, prune_annotations=None, padding=None))]
    pub fn redact_text(
        &mut self,
        query: String,
        case_sensitive: Option<bool>,
        page_indices: Option<Vec<usize>>,
        fill_color: Option<(f64, f64, f64)>,
        overlay_text: Option<String>,
        text_color: Option<(f64, f64, f64)>,
        font_size: Option<f64>,
        prune_annotations: Option<bool>,
        padding: Option<f64>,
    ) -> PyResult<Vec<PyRedactionSummary>> {
        gate(self.doc.require_modify())?;
        let pattern = RedactionPattern::Text {
            query,
            case_sensitive: case_sensitive.unwrap_or(false),
        };

        let mut config = RedactionConfig::default();
        if let Some((r, g, b)) = fill_color {
            config.fill_color = [r, g, b];
        }
        if let Some(text) = overlay_text {
            config.overlay_text = Some(text);
        }
        if let Some((r, g, b)) = text_color {
            config.text_color = [r, g, b];
        }
        if let Some(sz) = font_size {
            config.font_size = Some(sz);
        }
        if let Some(prune) = prune_annotations {
            config.prune_annotations = prune;
        }
        if let Some(pad) = padding {
            config.padding = pad;
        }

        let zero_indices: Option<Vec<usize>> = page_indices.map(|indices| {
            indices
                .into_iter()
                .map(|idx| {
                    if idx > 0 && idx <= self.page_ids.len() {
                        idx - 1
                    } else {
                        idx
                    }
                })
                .collect()
        });

        let summaries = pdf_engine_core::redact::redact_document_pattern(
            &mut self.doc,
            &pattern,
            zero_indices.as_deref(),
            &config,
        )
        .map_err(|e| PyRuntimeError::new_err(format!("Text redaction failed: {}", e)))?;

        self.reload_active_pages()?;
        Ok(summaries
            .into_iter()
            .map(PyRedactionSummary::from_core)
            .collect())
    }

    /// Scrubs sensitive metadata from the document (/Info dictionary and /Metadata XMP).
    pub fn sanitize_document(&mut self) -> PyResult<bool> {
        gate(self.doc.require_modify())?;
        let modified = pdf_engine_core::redact::scrub_document_metadata(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Sanitization failed: {}", e)))?;
        Ok(modified)
    }

    /// Encrypts the PDF with the standard security handler (Revision 4 AES-128 or Revisions 5/6 AES-256).
    /// `owner_password` is required. An empty owner password is rejected.
    #[pyo3(signature = (user_password="", owner_password="", permissions=None, encrypt_metadata=true, algorithm="aes128"))]
    pub fn encrypt(
        &mut self,
        user_password: &str,
        owner_password: &str,
        permissions: Option<PyPdfPermissions>,
        encrypt_metadata: bool,
        algorithm: &str,
    ) -> PyResult<()> {
        gate(self.doc.require_security_change())?;
        let revision = EncryptionRevision::from_name(algorithm).ok_or_else(|| {
            PyValueError::new_err(format!(
                "Unsupported encryption algorithm '{}'. Use 'aes128', 'aes256', or 'aes256-r5'.",
                algorithm
            ))
        })?;
        let perms = permissions.map(|p| p.to_core()).unwrap_or_default();
        let options = EncryptionOptions {
            user_password: user_password.to_string(),
            owner_password: owner_password.to_string(),
            permissions: perms,
            revision,
            encrypt_metadata,
        };
        pdf_engine_core::security::encrypt_document(&mut self.doc, &options)
            .map_err(|e| PyRuntimeError::new_err(format!("Encryption failed: {}", e)))?;
        Ok(())
    }

    /// Decrypts the PDF document in place using the supplied password.
    pub fn decrypt(&mut self, password: &str) -> PyResult<()> {
        pdf_engine_core::security::decrypt_document(&mut self.doc, password)
            .map_err(|e| PyRuntimeError::new_err(format!("Decryption failed: {}", e)))?;
        self.reload_active_pages()?;
        Ok(())
    }

    /// Returns true if the document has an `/Encrypt` trailer dictionary.
    pub fn is_encrypted(&self) -> bool {
        self.doc.xref.trailer.contains_key("Encrypt")
    }

    /// Returns the encryption algorithm label (`"AES-128-R4"`, `"AES-256-R5"`, or `"AES-256-R6"`) if encrypted.
    pub fn encryption_algorithm(&mut self) -> Option<String> {
        pdf_engine_core::security::get_encryption_revision(&mut self.doc)
            .map(|r| r.as_str().to_string())
    }

    /// Stamps a signature on one page.
    ///
    /// Without certificate material this is a SHA-256 byte-range attestation.
    /// With PEM or PKCS#12 material it writes `/SubFilter /adbe.pkcs7.detached`
    /// (RSA-SHA256 or P-256 ECDSA). Verification checks the certificate inside
    /// the CMS. It does not decide whether a viewer trusts that certificate.
    #[pyo3(signature = (
        signer_name,
        reason,
        location,
        rect,
        page_number=1,
        contact_info=None,
        certificate_pem=None,
        private_key_pem=None,
        chain_pem=None,
        pkcs12_der=None,
        pkcs12_password=None,
        timestamp_token=None,
        reserve_timestamp=false
    ))]
    pub fn sign(
        &mut self,
        signer_name: &str,
        reason: &str,
        location: &str,
        rect: [f64; 4],
        page_number: usize,
        contact_info: Option<String>,
        certificate_pem: Option<String>,
        private_key_pem: Option<String>,
        chain_pem: Option<String>,
        pkcs12_der: Option<Vec<u8>>,
        pkcs12_password: Option<Vec<u8>>,
        timestamp_token: Option<Vec<u8>>,
        reserve_timestamp: bool,
    ) -> PyResult<PyVerifiedSignature> {
        gate(self.doc.require_modify())?;
        let config = DigitalSignatureConfig {
            signer_name: signer_name.to_string(),
            reason: reason.to_string(),
            location: location.to_string(),
            rect,
            page_number,
            visual_badge: true,
            contact_info,
        };
        let cms = certificate_pem.is_some()
            || private_key_pem.is_some()
            || chain_pem.is_some()
            || pkcs12_der.is_some()
            || pkcs12_password.is_some()
            || timestamp_token.is_some()
            || reserve_timestamp;
        let sig = if cms {
            let material = pdf_engine_core::security::CmsSigningMaterial {
                certificate_pem: certificate_pem.as_deref(),
                private_key_pem: private_key_pem.as_deref(),
                chain_pem: chain_pem.as_deref(),
                pkcs12_der: pkcs12_der.as_deref(),
                pkcs12_password: pkcs12_password.as_deref(),
                timestamp_token: timestamp_token.as_deref(),
                reserve_timestamp,
            };
            pdf_engine_core::security::sign_document_cms(&mut self.doc, &config, &material)
        } else {
            pdf_engine_core::security::sign_document(&mut self.doc, &config)
        }
        .map_err(|e| PyRuntimeError::new_err(format!("Signing failed: {}", e)))?;
        self.reload_active_pages()?;
        Ok(PyVerifiedSignature::from_core(sig))
    }

    /// DER TimeStampReq over the signature value of the last valid PKCS#7.
    pub fn cms_timestamp_request(&self) -> PyResult<Vec<u8>> {
        pdf_engine_core::security::cms_timestamp_request(&self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Timestamp request failed: {}", e)))
    }

    /// Embeds an RFC 3161 token into the existing PKCS#7 contents hole.
    pub fn embed_cms_timestamp(&mut self, token: &[u8]) -> PyResult<PyVerifiedSignature> {
        gate(self.doc.require_modify())?;
        let sig = pdf_engine_core::security::embed_cms_timestamp(&mut self.doc, token)
            .map_err(|e| PyRuntimeError::new_err(format!("Timestamp embed failed: {}", e)))?;
        self.reload_active_pages()?;
        Ok(PyVerifiedSignature::from_core(sig))
    }

    /// Replaces the in-memory document with `bytes`, keeping the current limits.
    pub fn replace_bytes(&mut self, bytes: &[u8]) -> PyResult<()> {
        let limits = self.doc.limits.clone();
        let loaded = PdfDocument::load_with_limits(bytes, limits)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to load PDF: {}", e)))?;
        let rebuilt = Self::from_doc(loaded)?;
        self.doc = rebuilt.doc;
        self.page_ids = rebuilt.page_ids;
        self.active_pages = rebuilt.active_pages;
        Ok(())
    }

    /// Returns the stored file bytes without rewriting them.
    pub fn raw_bytes(&self) -> Vec<u8> {
        self.doc.raw_data().to_vec()
    }

    /// Reads signature dictionaries and checks each one.
    ///
    /// `PDFEngine.sha256` is compared with the byte-range digest.
    /// `adbe.pkcs7.detached` is checked against the certificate inside the CMS.
    /// That check is not a trust decision.
    pub fn verify_signatures(&self) -> PyResult<Vec<PyVerifiedSignature>> {
        let sigs = pdf_engine_core::security::verify_document_signatures(&self.doc);
        Ok(sigs
            .into_iter()
            .map(PyVerifiedSignature::from_core)
            .collect())
    }

    /// Detects all structured tables on the specified page (1-indexed).
    pub fn extract_tables(&mut self, page_number: usize) -> PyResult<Vec<PyDetectedTable>> {
        gate(self.doc.require_copy())?;
        let tables = pdf_engine_core::tables::detect_tables(&mut self.doc, page_number)
            .map_err(|e| PyRuntimeError::new_err(format!("Table extraction failed: {}", e)))?;
        Ok(tables.into_iter().map(PyDetectedTable::from_core).collect())
    }

    /// Exports a detected table into the specified format ("csv", "json", "markdown", "html").
    pub fn export_table(
        &mut self,
        page_number: usize,
        table_idx: usize,
        format: &str,
    ) -> PyResult<String> {
        gate(self.doc.require_copy())?;
        let tables = pdf_engine_core::tables::detect_tables(&mut self.doc, page_number)
            .map_err(|e| PyRuntimeError::new_err(format!("Table extraction failed: {}", e)))?;
        let table = tables.get(table_idx).ok_or_else(|| {
            PyValueError::new_err(format!(
                "Table index {} not found on page {}",
                table_idx, page_number
            ))
        })?;

        let export_format = match format.to_lowercase().as_str() {
            "csv" => pdf_engine_core::tables::TableExportFormat::Csv,
            "json" => pdf_engine_core::tables::TableExportFormat::Json,
            "markdown" | "md" => pdf_engine_core::tables::TableExportFormat::Markdown,
            "html" => pdf_engine_core::tables::TableExportFormat::Html,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "Unsupported export format '{}'. Supported: csv, json, markdown, html",
                    format
                )))
            }
        };

        Ok(pdf_engine_core::tables::export_table(table, export_format))
    }

    /// Saves the modified PDF document to a filesystem path.
    pub fn save(&mut self, path: &str) -> PyResult<()> {
        let bytes = self.save_to_bytes()?;
        fs::write(path, bytes)
            .map_err(|e| PyIOError::new_err(format!("Failed to write to '{}': {}", path, e)))?;
        Ok(())
    }

    /// Returns document metadata as a dictionary (/Info and XMP synchronized).
    pub fn get_metadata<'py>(
        &mut self,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let meta = self
            .doc
            .get_metadata()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract metadata: {}", e)))?;
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("title", meta.title)?;
        dict.set_item("author", meta.author)?;
        dict.set_item("subject", meta.subject)?;
        dict.set_item("keywords", meta.keywords)?;
        dict.set_item("creator", meta.creator)?;
        dict.set_item("producer", meta.producer)?;
        dict.set_item("creation_date", meta.creation_date)?;
        dict.set_item("mod_date", meta.mod_date)?;
        Ok(dict)
    }

    /// Updates and synchronizes document metadata across /Info and XMP stream.
    pub fn set_metadata(&mut self, data: &Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        gate(self.doc.require_modify())?;
        let current = self.doc.get_metadata().unwrap_or_default();
        let resolve_field = |key: &str, current_val: Option<String>| -> PyResult<Option<String>> {
            if data.contains(key)? {
                match data.get_item(key)? {
                    Some(val) => {
                        if val.is_none() {
                            Ok(None)
                        } else {
                            Ok(val.extract::<String>().ok())
                        }
                    }
                    None => Ok(None),
                }
            } else {
                Ok(current_val)
            }
        };

        let meta = pdf_engine_core::ops::DocumentMetadata {
            title: resolve_field("title", current.title)?,
            author: resolve_field("author", current.author)?,
            subject: resolve_field("subject", current.subject)?,
            keywords: resolve_field("keywords", current.keywords)?,
            creator: resolve_field("creator", current.creator)?,
            producer: resolve_field("producer", current.producer)?,
            creation_date: resolve_field("creation_date", current.creation_date)?,
            mod_date: resolve_field("mod_date", current.mod_date)?,
        };

        self.doc
            .set_metadata(&meta)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to update metadata: {}", e)))?;
        Ok(())
    }

    /// Serializes the modified PDF document to an in-memory byte vector.
    pub fn save_to_bytes(&mut self) -> PyResult<Vec<u8>> {
        self.doc
            .save_to_vec()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to serialize PDF: {}", e)))
    }

    /// Exports the PDF document into a native Microsoft Word OpenXML (`.docx`) byte buffer.
    pub fn export_docx(&mut self) -> PyResult<Vec<u8>> {
        gate(self.doc.require_copy())?;
        pdf_engine_core::export_document_docx(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to export DOCX: {}", e)))
    }

    /// Exports detected tables into an Office Open XML workbook (`.xlsx`).
    ///
    /// Each table is one worksheet. Every cell is reconstructed text, so a value
    /// that looks like a number or a formula stays text. This is not a binary `.xls`.
    pub fn export_xlsx(&mut self) -> PyResult<Vec<u8>> {
        gate(self.doc.require_copy())?;
        pdf_engine_core::export_document_xlsx(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to export XLSX: {}", e)))
    }

    /// Exports each page as one PowerPoint slide (`.pptx`).
    ///
    /// The deck size is the first page's displayed box. A later page is stretched
    /// onto that size. A paragraph is a text box. A table is reconstructed text.
    /// This is not a binary `.ppt`.
    pub fn export_pptx(&mut self) -> PyResult<Vec<u8>> {
        gate(self.doc.require_copy())?;
        pdf_engine_core::export_document_pptx(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to export PPTX: {}", e)))
    }

    /// Exports the PDF document into structured GitHub-Flavored Markdown (`.md`).
    pub fn export_markdown(&mut self) -> PyResult<String> {
        gate(self.doc.require_copy())?;
        pdf_engine_core::export_document_markdown(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to export Markdown: {}", e)))
    }

    /// Optimizes the PDF document in-place using reachability analysis, stream recompression,
    /// stream deduplication, and Object Stream (/ObjStm) packing.
    #[pyo3(signature = (
        remove_unused = true,
        pack_object_streams = true,
        recompress_flate = true,
        deduplicate_streams = true,
        max_objects_per_stream = 100
    ))]
    pub fn optimize(
        &mut self,
        remove_unused: bool,
        pack_object_streams: bool,
        recompress_flate: bool,
        deduplicate_streams: bool,
        max_objects_per_stream: usize,
    ) -> PyResult<PyOptimizationStats> {
        gate(self.doc.require_modify())?;
        let options = pdf_engine_core::ops::OptimizationOptions {
            remove_unused_objects: remove_unused,
            pack_into_object_streams: pack_object_streams,
            max_objects_per_stream,
            recompress_flate,
            deduplicate_streams,
            use_xref_stream: true,
        };
        let stats = self
            .doc
            .optimize(&options)
            .map_err(|e| PyRuntimeError::new_err(format!("Optimization failed: {}", e)))?;
        self.reload_active_pages()?;
        Ok(PyOptimizationStats::from_core(stats))
    }

    /// Saves the optimized PDF document to an in-memory byte buffer and returns (bytes, stats).
    #[pyo3(signature = (
        remove_unused = true,
        pack_object_streams = true,
        recompress_flate = true,
        deduplicate_streams = true,
        max_objects_per_stream = 100
    ))]
    pub fn save_optimized_to_bytes(
        &mut self,
        remove_unused: bool,
        pack_object_streams: bool,
        recompress_flate: bool,
        deduplicate_streams: bool,
        max_objects_per_stream: usize,
    ) -> PyResult<(Vec<u8>, PyOptimizationStats)> {
        gate(self.doc.require_modify())?;
        let options = pdf_engine_core::ops::OptimizationOptions {
            remove_unused_objects: remove_unused,
            pack_into_object_streams: pack_object_streams,
            max_objects_per_stream,
            recompress_flate,
            deduplicate_streams,
            use_xref_stream: true,
        };
        let (bytes, stats) = self.doc.save_optimized_to_vec(&options).map_err(|e| {
            PyRuntimeError::new_err(format!("Optimization serialization failed: {}", e))
        })?;
        Ok((bytes, PyOptimizationStats::from_core(stats)))
    }

    /// Compares this PDF document (as base) against a target document revision.
    ///
    /// Returns a structured comparison report dictionary with summary counts,
    /// metadata discrepancies, and per-page textual and image differences.
    #[pyo3(signature = (
        target,
        ignore_case = false,
        similarity_threshold = 0.35,
        compare_images = true,
        compare_metadata = true
    ))]
    pub fn compare<'py>(
        &mut self,
        py: Python<'py>,
        target: &mut PyPdfDocument,
        ignore_case: bool,
        similarity_threshold: f64,
        compare_images: bool,
        compare_metadata: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        gate(self.doc.require_copy())?;
        gate(target.doc.require_copy())?;
        let options = pdf_engine_core::ops::DiffOptions {
            ignore_whitespace: true,
            ignore_case,
            similarity_threshold,
            compare_images,
            compare_metadata,
        };
        let report =
            pdf_engine_core::ops::compare_documents(&mut self.doc, &mut target.doc, &options)
                .map_err(|e| PyRuntimeError::new_err(format!("Comparison failed: {}", e)))?;
        let json_str = report.to_json();
        let json_mod = py.import("json")?;
        json_mod.call_method1("loads", (json_str,))
    }

    /// Extracts the hierarchical Document Outline (`/Outlines` / Table of Contents) tree.
    pub fn get_outlines(&mut self) -> PyResult<Vec<PyOutlineItem>> {
        let items = self
            .doc
            .get_outlines()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract outlines: {}", e)))?;
        Ok(items.into_iter().map(PyOutlineItem::from_core).collect())
    }

    /// Replaces the Document Outline (`/Outlines` / Table of Contents) tree and returns total items written.
    pub fn set_outlines(&mut self, items: Vec<PyOutlineItem>) -> PyResult<usize> {
        gate(self.doc.require_assemble())?;
        let core_items: Vec<pdf_engine_core::ops::OutlineItem> =
            items.iter().map(PyOutlineItem::to_core).collect();
        self.doc
            .set_outlines(&core_items)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to update outlines: {}", e)))
    }

    /// Appends a single outline item at root or under `parent_indices` and returns total items written.
    #[pyo3(signature = (item, parent_indices=None))]
    pub fn add_outline_item(
        &mut self,
        item: PyOutlineItem,
        parent_indices: Option<Vec<usize>>,
    ) -> PyResult<usize> {
        gate(self.doc.require_assemble())?;
        let indices = parent_indices.unwrap_or_default();
        pdf_engine_core::ops::add_outline_item(&mut self.doc, item.to_core(), &indices)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to add outline item: {}", e)))
    }

    /// Removes the entire `/Outlines` tree from the document.
    pub fn clear_outlines(&mut self) -> PyResult<bool> {
        gate(self.doc.require_assemble())?;
        pdf_engine_core::ops::clear_outlines(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to clear outlines: {}", e)))
    }

    /// Lists all embedded file attachments in the document (`/Names /EmbeddedFiles` and `/FileAttachment` annotations).
    pub fn list_attachments(&mut self) -> PyResult<Vec<PyEmbeddedAttachmentInfo>> {
        let items = pdf_engine_core::ops::list_attachments(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to list attachments: {}", e)))?;
        Ok(items
            .into_iter()
            .map(PyEmbeddedAttachmentInfo::from_core)
            .collect())
    }

    /// Extracts a specific embedded file attachment by logical name or filename.
    pub fn extract_attachment(&mut self, name: &str) -> PyResult<PyEmbeddedAttachment> {
        gate(self.doc.require_copy())?;
        let att = pdf_engine_core::ops::extract_attachment(&mut self.doc, name)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract attachment: {}", e)))?;
        Ok(PyEmbeddedAttachment::from_core(att))
    }

    /// Embeds a file into the document's `/Names /EmbeddedFiles` name tree (ISO 32000-1 §7.11.4).
    #[pyo3(signature = (name, data, mime_type=None, description=None))]
    pub fn add_attachment(
        &mut self,
        name: &str,
        data: &[u8],
        mime_type: Option<&str>,
        description: Option<&str>,
    ) -> PyResult<PyEmbeddedAttachmentInfo> {
        gate(self.doc.require_modify())?;
        let info =
            pdf_engine_core::ops::add_attachment(&mut self.doc, name, data, mime_type, description)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to add attachment: {}", e)))?;
        Ok(PyEmbeddedAttachmentInfo::from_core(info))
    }

    /// Deletes an embedded file attachment by logical name or filename.
    pub fn delete_attachment(&mut self, name: &str) -> PyResult<bool> {
        gate(self.doc.require_modify())?;
        pdf_engine_core::ops::delete_attachment(&mut self.doc, name)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete attachment: {}", e)))
    }

    /// Extracts all Optional Content Groups (`/OCProperties` / PDF Layers, ISO 32000-1 §8.11) from the document.
    pub fn get_layers(&mut self) -> PyResult<Vec<PyOptionalContentLayer>> {
        let layers = pdf_engine_core::ops::extract_layers(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract layers: {}", e)))?;
        Ok(layers
            .into_iter()
            .map(PyOptionalContentLayer::from_core)
            .collect())
    }

    /// Creates and registers a new Optional Content Group (`/Type /OCG` — PDF Layer) in the document.
    #[pyo3(signature = (name, visible=true, intent=None, locked=false))]
    pub fn add_layer(
        &mut self,
        name: &str,
        visible: bool,
        intent: Option<Vec<String>>,
        locked: bool,
    ) -> PyResult<PyOptionalContentLayer> {
        gate(self.doc.require_modify())?;
        let layer = pdf_engine_core::ops::add_layer(&mut self.doc, name, visible, intent, locked)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to add layer: {}", e)))?;
        self.reload_active_pages()?;
        Ok(PyOptionalContentLayer::from_core(layer))
    }

    /// Toggles the visibility (`ON` / `OFF`) of an Optional Content Group by `id` (e.g. `"ocg_12"`) or `name`
    /// and refreshes active page scene graphs so hidden layer content is immediately filtered.
    pub fn set_layer_visibility(
        &mut self,
        target: &str,
        visible: bool,
    ) -> PyResult<PyOptionalContentLayer> {
        gate(self.doc.require_modify())?;
        let updated = pdf_engine_core::ops::set_layer_visibility(&mut self.doc, target, visible)
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to set layer visibility: {}", e))
            })?;
        self.reload_active_pages()?;
        Ok(PyOptionalContentLayer::from_core(updated))
    }

    /// Deletes an Optional Content Group (`/Type /OCG`) by `id` or `name` from the document's `/OCProperties`.
    pub fn delete_layer(&mut self, target: &str) -> PyResult<bool> {
        gate(self.doc.require_modify())?;
        let deleted = pdf_engine_core::ops::delete_layer(&mut self.doc, target)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to delete layer: {}", e)))?;
        if deleted {
            self.reload_active_pages()?;
        }
        Ok(deleted)
    }

    /// Binds a marked-content property name (e.g. `"MC0"` or `"L1"` used in `/OC /L1 BDC ... EMC`)
    /// in a 1-indexed page's `/Resources /Properties` dictionary to an existing OCG layer (`id` or `name`).
    pub fn register_page_layer_property(
        &mut self,
        page_number: usize,
        prop_name: &str,
        layer_target: &str,
    ) -> PyResult<String> {
        gate(self.doc.require_modify())?;
        let layer_id = pdf_engine_core::ops::register_page_layer_property(
            &mut self.doc,
            page_number,
            prop_name,
            layer_target,
        )
        .map_err(|e| {
            PyRuntimeError::new_err(format!("Failed to register page layer property: {}", e))
        })?;
        self.reload_active_pages()?;
        Ok(layer_id)
    }

    /// Permanently flattens all (or selected 1-based `page_numbers`) interactive AcroForm fields,
    /// visual annotations, rubber stamps, vector shapes, and `/AP /N` appearance streams directly
    /// into page content streams, and purges `/Annots` and `/AcroForm` dictionaries from the document.
    #[pyo3(signature = (page_numbers=None))]
    pub fn flatten_permanently(
        &mut self,
        page_numbers: Option<Vec<usize>>,
    ) -> PyResult<PyHardFlattenReport> {
        gate(self.doc.require_modify())?;
        let zero_indices: Option<Vec<usize>> = match page_numbers {
            Some(nums) => {
                let mut v = Vec::with_capacity(nums.len());
                for n in nums {
                    if n == 0 || n > self.page_ids.len() {
                        return Err(PyValueError::new_err(format!(
                            "Page number {} out of range (1..={})",
                            n,
                            self.page_ids.len()
                        )));
                    }
                    v.push(n - 1);
                }
                Some(v)
            }
            None => None,
        };

        let report = pdf_engine_core::ops::flatten_document_permanently(
            &mut self.doc,
            zero_indices.as_deref(),
        )
        .map_err(|e| PyRuntimeError::new_err(format!("Hard flatten failed: {}", e)))?;

        self.reload_active_pages()?;
        Ok(PyHardFlattenReport::from_core(report))
    }

    /// Inspects the logical structure tree (`/StructTreeRoot`) and validates ISO 14289-1 (PDF/UA-1) accessibility compliance.
    pub fn inspect_pdfua<'py>(
        &mut self,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let report = pdf_engine_core::inspect_pdfua(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("PDF/UA inspection failed: {}", e)))?;
        pdfua_report_to_py_dict(py, &report)
    }

    /// Automatically generates a `/StructTreeRoot` logical structure hierarchy, `/RoleMap`,
    /// `/MarkInfo`, `/ViewerPreferences`, `/Lang`, and `pdfuaid:part` XMP metadata stream
    /// to upgrade the document to ISO 14289-1 (PDF/UA-1) compliance.
    #[pyo3(signature = (
        lang = "en-US",
        title = None,
        figure_alt_fallback = "Document figure or illustration"
    ))]
    pub fn apply_pdfua_structure<'py>(
        &mut self,
        py: Python<'py>,
        lang: &str,
        title: Option<String>,
        figure_alt_fallback: &str,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_modify())?;
        let options = pdf_engine_core::PdfUaTaggingOptions {
            lang: lang.to_string(),
            title,
            figure_alt_fallback: figure_alt_fallback.to_string(),
        };
        let report = pdf_engine_core::apply_pdfua_structure(&mut self.doc, &options)
            .map_err(|e| PyRuntimeError::new_err(format!("PDF/UA tagging failed: {}", e)))?;
        self.reload_active_pages()?;
        pdfua_report_to_py_dict(py, &report)
    }

    /// Inserts an empty page at a 0-based index. The current page count appends.
    ///
    /// Width and height are points inside `(0, 14400]`. Returns the landing index.
    #[pyo3(signature = (index, width=612.0, height=792.0))]
    pub fn insert_blank_page(&mut self, index: usize, width: f64, height: f64) -> PyResult<usize> {
        gate(self.doc.require_assemble())?;
        let landed = pdf_engine_core::ops::insert_blank_page(&mut self.doc, index, width, height)
            .map_err(map_engine_value)?;
        self.refresh_handles()?;
        Ok(landed)
    }

    /// Copies one page inside this document. Both indexes are 0-based.
    ///
    /// `insert_at` equal to the page count appends. Returns the landing index.
    pub fn duplicate_page(&mut self, source_index: usize, insert_at: usize) -> PyResult<usize> {
        gate(self.doc.require_assemble())?;
        let landed =
            pdf_engine_core::ops::duplicate_page(&mut self.doc, source_index, insert_at)
                .map_err(map_engine_value)?;
        self.refresh_handles()?;
        Ok(landed)
    }

    /// Copies pages from `source` into this document at a 0-based index.
    ///
    /// `source` must be a different document. A copy inside this document uses
    /// `duplicate_page`. Returns how many pages were inserted.
    pub fn insert_pages(
        &mut self,
        source: &PyPdfDocument,
        source_indices: Vec<usize>,
        insert_at: usize,
    ) -> PyResult<usize> {
        gate(self.doc.require_assemble())?;
        gate(source.doc.require_copy())?;
        let mut source_doc = source.doc.clone();
        let inserted = pdf_engine_core::ops::insert_pages(
            &mut self.doc,
            &mut source_doc,
            &source_indices,
            insert_at,
        )
        .map_err(map_engine_value)?;
        self.refresh_handles()?;
        Ok(inserted)
    }

    /// Reverses page order. Fewer than two pages leaves the document unchanged.
    pub fn reverse_pages(&mut self) -> PyResult<()> {
        gate(self.doc.require_assemble())?;
        pdf_engine_core::ops::reverse_pages(&mut self.doc).map_err(map_engine_value)?;
        self.refresh_handles()?;
        Ok(())
    }

    /// Reads the five ISO 32000-1 page boxes. `page_index` is 0-based.
    ///
    /// Missing optional boxes are `None`. A missing MediaBox is reported as US Letter.
    pub fn page_boxes<'py>(
        &mut self,
        py: Python<'py>,
        page_index: usize,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let boxes = pdf_engine_core::ops::page_boxes(&mut self.doc, page_index)
            .map_err(map_engine_value)?;
        page_boxes_to_py(py, &boxes)
    }

    /// Writes or clears one page box on that page. `page_index` is 0-based.
    ///
    /// `corners` is `[x0, y0, x1, y1]`. `None` removes crop, bleed, trim, or art
    /// from this page only. MediaBox cannot be removed.
    #[pyo3(signature = (page_index, kind, corners=None))]
    pub fn set_page_box<'py>(
        &mut self,
        py: Python<'py>,
        page_index: usize,
        kind: &str,
        corners: Option<Vec<f64>>,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_modify())?;
        let boxes = pdf_engine_core::ops::set_page_boundary(
            &mut self.doc,
            page_index,
            kind,
            corners.as_deref(),
        )
        .map_err(map_engine_value)?;
        page_boxes_to_py(py, &boxes)
    }

    /// Searches reconstructed paragraph text. The first page is index 0.
    ///
    /// Offsets count Unicode scalars in the paragraph. The query is not stored.
    #[pyo3(signature = (query, ignore_case=false, max_hits=100))]
    pub fn search_text<'py>(
        &mut self,
        py: Python<'py>,
        query: &str,
        ignore_case: bool,
        max_hits: usize,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_copy())?;
        let report =
            pdf_engine_core::ops::search_document(&mut self.doc, query, ignore_case, max_hits)
                .map_err(map_engine_value)?;
        search_report_to_py(py, &report)
    }

    /// Rewrites reconstructed paragraph text.
    ///
    /// A subset stays only when every character of the new paragraph has a real
    /// width. Otherwise that paragraph moves to Helvetica, Times, or Courier.
    /// A character those faces cannot draw changes nothing. The query and the
    /// replacement are not stored.
    #[pyo3(signature = (query, replacement, ignore_case=true, max_hits=100))]
    pub fn replace_text<'py>(
        &mut self,
        py: Python<'py>,
        query: &str,
        replacement: &str,
        ignore_case: bool,
        max_hits: usize,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_modify())?;
        let report = pdf_engine_core::ops::replace_document_text(
            &mut self.doc,
            query,
            replacement,
            ignore_case,
            max_hits,
        )
        .map_err(map_engine_value)?;
        self.refresh_handles()?;
        replace_report_to_py(py, &report)
    }

    /// Paints one page to PNG, JPEG, or TIFF. `page_index` is 0-based.
    ///
    /// This is a read. It does not rewrite the page. It is not an Acrobat
    /// rendering: annotation appearances are not painted, and rendering mode 3
    /// is not painted. Text without outlines uses a built-in face.
    #[pyo3(signature = (page_index, format="png", dpi=144.0))]
    pub fn render_page<'py>(
        &mut self,
        py: Python<'py>,
        page_index: usize,
        format: &str,
        dpi: f64,
    ) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        gate(self.doc.require_print())?;
        let parsed =
            pdf_engine_core::ops::RasterFormat::parse(format).map_err(map_engine_value)?;
        let raster =
            pdf_engine_core::ops::render_page(&mut self.doc, page_index, parsed, dpi)
                .map_err(map_engine_value)?;
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("bytes", pyo3::types::PyBytes::new(py, &raster.bytes))?;
        dict.set_item("width", raster.width)?;
        dict.set_item("height", raster.height)?;
        dict.set_item("dpi", raster.dpi)?;
        dict.set_item("omitted", raster.omitted)?;
        Ok(dict)
    }
}

fn fill_sign_input(message: &str) -> PyErr {
    PyValueError::new_err(format!(
        "Document operation error: Fill and sign: {message}"
    ))
}

fn map_engine_value(err: pdf_engine_core::PdfError) -> PyErr {
    match &err {
        pdf_engine_core::PdfError::OperationError(_)
        | pdf_engine_core::PdfError::InvalidPageNumber { .. } => {
            PyValueError::new_err(err.to_string())
        }
        _ => PyRuntimeError::new_err(err.to_string()),
    }
}

fn gate(check: pdf_engine_core::PdfResult<()>) -> PyResult<()> {
    check.map_err(map_engine_value)
}

fn permission_denied() -> PyErr {
    map_engine_value(pdf_engine_core::PdfError::CryptographyError(
        pdf_engine_core::security::permissions::PERMISSION_DENIED.to_string(),
    ))
}

fn rect_corners(rect: Rect) -> [f64; 4] {
    [rect.min_x, rect.min_y, rect.max_x, rect.max_y]
}

fn page_boxes_to_py<'py>(
    py: Python<'py>,
    boxes: &pdf_engine_core::ops::PageBoxes,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("media", rect_corners(boxes.media))?;
    dict.set_item("crop", boxes.crop.map(rect_corners))?;
    dict.set_item("bleed", boxes.bleed.map(rect_corners))?;
    dict.set_item("trim", boxes.trim.map(rect_corners))?;
    dict.set_item("art", boxes.art.map(rect_corners))?;
    Ok(dict)
}

fn search_report_to_py<'py>(
    py: Python<'py>,
    report: &pdf_engine_core::ops::TextSearchReport,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let hits = pyo3::types::PyList::empty(py);
    for hit in &report.hits {
        let item = pyo3::types::PyDict::new(py);
        item.set_item("page_index", hit.page_index)?;
        item.set_item("paragraph_id", hit.paragraph_id)?;
        item.set_item("char_start", hit.char_start)?;
        item.set_item("char_end", hit.char_end)?;
        item.set_item("snippet", &hit.snippet)?;
        item.set_item("min_x", hit.min_x)?;
        item.set_item("min_y", hit.min_y)?;
        item.set_item("max_x", hit.max_x)?;
        item.set_item("max_y", hit.max_y)?;
        hits.append(item)?;
    }
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("hits", hits)?;
    dict.set_item("truncated", report.truncated)?;
    dict.set_item("pages_skipped", &report.pages_skipped)?;
    Ok(dict)
}

fn replace_report_to_py<'py>(
    py: Python<'py>,
    report: &pdf_engine_core::ops::TextReplaceReport,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("replacements", report.replacements)?;
    dict.set_item("paragraphs", report.paragraphs)?;
    dict.set_item(
        "paragraphs_on_standard_face",
        report.paragraphs_on_standard_face,
    )?;
    dict.set_item("truncated", report.truncated)?;
    dict.set_item("pages_skipped", &report.pages_skipped)?;
    Ok(dict)
}

fn structure_element_to_py_dict<'py>(
    py: Python<'py>,
    el: &pdf_engine_core::StructureElement,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let d = pyo3::types::PyDict::new(py);
    d.set_item("struct_type", &el.struct_type)?;
    d.set_item("title", &el.title)?;
    d.set_item("alt_text", &el.alt_text)?;
    d.set_item("actual_text", &el.actual_text)?;
    d.set_item("lang", &el.lang)?;
    d.set_item("page_number", el.page_number)?;
    d.set_item("mcids", &el.mcids)?;

    let children_list = pyo3::types::PyList::empty(py);
    for child in &el.children {
        children_list.append(structure_element_to_py_dict(py, child)?)?;
    }
    d.set_item("children", children_list)?;
    Ok(d)
}

fn pdfua_report_to_py_dict<'py>(
    py: Python<'py>,
    report: &pdf_engine_core::PdfUaReport,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let d = pyo3::types::PyDict::new(py);
    d.set_item("is_compliant", report.is_compliant)?;
    d.set_item("is_tagged", report.is_tagged)?;
    d.set_item("lang", &report.lang)?;
    d.set_item("title", &report.title)?;
    d.set_item("display_doc_title", report.display_doc_title)?;
    d.set_item("has_pdfua_xmp", report.has_pdfua_xmp)?;

    let rm_dict = pyo3::types::PyDict::new(py);
    for (k, v) in &report.role_map {
        rm_dict.set_item(k, v)?;
    }
    d.set_item("role_map", rm_dict)?;
    d.set_item("total_elements", report.total_elements)?;

    let elements_list = pyo3::types::PyList::empty(py);
    for el in &report.elements {
        elements_list.append(structure_element_to_py_dict(py, el)?)?;
    }
    d.set_item("elements", elements_list)?;

    let issues_list = pyo3::types::PyList::empty(py);
    for issue in &report.issues {
        let idict = pyo3::types::PyDict::new(py);
        idict.set_item("code", &issue.code)?;
        idict.set_item("description", &issue.description)?;
        idict.set_item("clause", &issue.clause)?;
        issues_list.append(idict)?;
    }
    d.set_item("issues", issues_list)?;
    Ok(d)
}

/// Merges multiple Python PDF documents sequentially into a single unified document.
#[pyfunction]
pub fn merge_documents(docs: Vec<PyRef<'_, PyPdfDocument>>) -> PyResult<PyPdfDocument> {
    for doc in &docs {
        gate(doc.doc.require_copy())?;
    }
    let mut raw_docs: Vec<PdfDocument> = docs.iter().map(|d| d.doc.clone()).collect();
    let merged = pdf_engine_core::ops::merge_documents(&mut raw_docs)
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to merge documents: {}", e)))?;
    PyPdfDocument::from_doc(merged)
}

/// Merges multiple raw PDF byte buffers into a single serialized PDF byte vector.
#[pyfunction]
pub fn merge_pdf_bytes(pdf_buffers: Vec<Vec<u8>>) -> PyResult<Vec<u8>> {
    let slices: Vec<&[u8]> = pdf_buffers.iter().map(|b| b.as_slice()).collect();
    pdf_engine_core::ops::merge_pdf_bytes(&slices)
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to merge PDF bytes: {}", e)))
}

/// Compares two PDF documents and returns a structured comparison report dictionary.
#[pyfunction]
#[pyo3(signature = (
    base,
    target,
    ignore_case = false,
    similarity_threshold = 0.35,
    compare_images = true,
    compare_metadata = true
))]
pub fn compare_documents<'py>(
    py: Python<'py>,
    base: &mut PyPdfDocument,
    target: &mut PyPdfDocument,
    ignore_case: bool,
    similarity_threshold: f64,
    compare_images: bool,
    compare_metadata: bool,
) -> PyResult<Bound<'py, PyAny>> {
    base.compare(
        py,
        target,
        ignore_case,
        similarity_threshold,
        compare_images,
        compare_metadata,
    )
}

/// The native Python module entrypoint for `pdf_engine`.
#[pymodule]
fn pdf_engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPdfDocument>()?;
    m.add_class::<PyPage>()?;
    m.add_class::<PyParagraph>()?;
    m.add_class::<PyImageInfo>()?;
    m.add_class::<PyFormField>()?;
    m.add_class::<PyDetectedFormFieldCandidate>()?;
    m.add_class::<PyAnnotation>()?;
    m.add_class::<PyRedactionSummary>()?;
    m.add_class::<PyPdfPermissions>()?;
    m.add_class::<PyVerifiedSignature>()?;
    m.add_class::<PyTableCell>()?;
    m.add_class::<PyDetectedTable>()?;
    m.add_class::<PyOptimizationStats>()?;
    m.add_class::<PyOutlineItem>()?;
    m.add_class::<PyEmbeddedAttachmentInfo>()?;
    m.add_class::<PyEmbeddedAttachment>()?;
    m.add_class::<PyOptionalContentLayer>()?;
    m.add_class::<PyHardFlattenReport>()?;
    m.add_function(wrap_pyfunction!(compare_documents, m)?)?;
    m.add_function(wrap_pyfunction!(merge_documents, m)?)?;
    m.add_function(wrap_pyfunction!(merge_pdf_bytes, m)?)?;
    Ok(())
}
