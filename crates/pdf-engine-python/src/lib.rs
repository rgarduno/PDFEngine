//! Python bindings for the PDFEngine core using PyO3.
//!
//! Exposes high-level document loading, page scene graph inspection,
//! and in-place surgical paragraph editing to Python and FastAPI backends.

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::fs;

use pdf_engine_core::annots::{
    AnnotationSubtype, LinkAction, StampType,
};
use pdf_engine_core::cos::{ObjectId, PdfDocument, PdfObject, PdfStream};
use pdf_engine_core::editor::SurgicalEditor;
use pdf_engine_core::fonts::FontMetrics;
use pdf_engine_core::layout::geometry::Rect;
use pdf_engine_core::layout::{LayoutReconstructor, ParagraphBlock, TextAlignment};
use pdf_engine_core::stream::{
    build_ast_from_operations, serialize_ast, ContentAst, ContentStreamTokenizer,
};
use pdf_engine_core::watermark::{
    ImageWatermarkConfig, PaginationConfig, PaginationPosition, TextWatermarkConfig,
    WatermarkPlacement,
};
use pdf_engine_core::redact::{
    RedactionConfig, RedactionPattern, RedactionSummary,
};
use pdf_engine_core::security::{
    DigitalSignatureConfig, EncryptionOptions, EncryptionRevision, PdfPermissions,
    VerifiedSignature,
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
}

#[pymethods]
impl PyParagraph {
    fn __repr__(&self) -> String {
        format!(
            "<Paragraph id={} lines={} align='{}' text='{:.30}...'>",
            self.id, self.line_count, self.alignment, self.text.replace('\n', " ")
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
            self.id, self.name, self.width_px, self.height_px, self.color_space,
            self.min_x, self.min_y, self.max_x, self.max_y
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
            self.page_number, self.purged_glyphs_count, self.blackout_boxes_count, self.pruned_annotations_count
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

/// Granular user access permissions matching ISO 32000-1 §7.6.3.2 Table 22.
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

/// Cryptographic verification data of an embedded digital signature.
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
}

#[pymethods]
impl PyPage {
    /// Returns the list of reconstructed paragraph blocks on this page.
    pub fn get_paragraphs(&self) -> Vec<PyParagraph> {
        self.paragraphs
            .iter()
            .map(|p| {
                let align_str = match p.alignment {
                    TextAlignment::Left => "left",
                    TextAlignment::Center => "center",
                    TextAlignment::Right => "right",
                    TextAlignment::Justified => "justified",
                };
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
                }
            })
            .collect()
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

        // Reconstruct layout after mutation to keep page state synchronized
        let reconstructor =
            LayoutReconstructor::new(&self.ast).with_font("F1", self.metrics.clone());
        self.paragraphs = reconstructor.reconstruct();

        Ok(())
    }

    /// Serializes the page content stream into raw bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        serialize_ast(&self.ast)
    }

    /// Returns a JSON-formatted SceneGraph of the page layout.
    pub fn scenegraph_json(&self) -> String {
        let paragraphs = self.get_paragraphs();
        let mut json = String::from("{\"page\":");
        json.push_str(&self.page_number.to_string());
        json.push_str(",\"paragraphs\":[");

        for (i, p) in paragraphs.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "{{\"id\":{},\"text\":{:?},\"bbox\":{{\"min_x\":{:.2},\"min_y\":{:.2},\"max_x\":{:.2},\"max_y\":{:.2}}},\"alignment\":\"{}\",\"leading\":{:.2},\"lines\":{}}}",
                p.id, p.text, p.min_x, p.min_y, p.max_x, p.max_y, p.alignment, p.leading, p.line_count
            ));
        }

        json.push_str("]}");
        json
    }
}

/// Primary PDF Document controller exposed to Python.
#[pyclass(name = "Document")]
pub struct PyPdfDocument {
    doc: PdfDocument,
    page_ids: Vec<ObjectId>,
    active_pages: Vec<PyPage>,
}

impl PyPdfDocument {
    /// Internal factory reconstructing active page scene graphs from a core PdfDocument.
    pub fn from_doc(mut doc: PdfDocument) -> PyResult<Self> {
        let page_ids = doc
            .get_pages()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to resolve pages: {}", e)))?;

        let mut active_pages = Vec::with_capacity(page_ids.len());
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);

        for (idx, &page_id) in page_ids.iter().enumerate() {
            let page_obj = doc
                .get_object(page_id)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to load page: {}", e)))?;

            let (contents_id, ast, paragraphs) = if let Some(dict) = page_obj.as_dict() {
                let c_ref = dict.get("Contents").and_then(|c| c.as_reference());
                if let Some(contents_ref) = c_ref {
                    let contents_obj = doc.get_object(contents_ref).map_err(|e| {
                        PyRuntimeError::new_err(format!("Failed to load stream: {}", e))
                    })?;

                    if let PdfObject::Stream(s) = contents_obj {
                        let mut tokenizer = ContentStreamTokenizer::new(&s.content);
                        let ops = tokenizer.tokenize_all().unwrap_or_default();
                        let ast = build_ast_from_operations(ops);
                        let reconstructor =
                            LayoutReconstructor::new(&ast).with_font("F1", metrics.clone());
                        let paragraphs = reconstructor.reconstruct();
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
            });
        }

        Ok(Self {
            doc,
            page_ids,
            active_pages,
        })
    }

    /// Reloads all active PyPage scene graphs to reflect recent stream mutations or flatten operations.
    pub(crate) fn reload_active_pages(&mut self) {
        let page_ids = self.page_ids.clone();
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
        let mut reloaded_pages = Vec::with_capacity(page_ids.len());

        for (idx, &page_id) in page_ids.iter().enumerate() {
            if let Ok(page_obj) = self.doc.get_object(page_id) {
                if let Some(dict) = page_obj.as_dict() {
                    let contents_id = dict.get("Contents").and_then(|c| c.as_reference());
                    let (ast, paragraphs) = if let Some(c_ref) = contents_id {
                        if let Ok(PdfObject::Stream(s)) = self.doc.get_object(c_ref) {
                            let mut tokenizer = ContentStreamTokenizer::new(&s.content);
                            let ops = tokenizer.tokenize_all().unwrap_or_default();
                            let ast = build_ast_from_operations(ops);
                            let reconstructor =
                                LayoutReconstructor::new(&ast).with_font("F1", metrics.clone());
                            let paragraphs = reconstructor.reconstruct();
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
                    });
                }
            }
        }
        self.active_pages = reloaded_pages;
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

    /// Retrieves a mutable reference to a page by 1-based or 0-based index.
    pub fn get_page(&mut self, index: usize) -> PyResult<PyPage> {
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
            self.active_pages[zero_idx] = PyPage {
                page_number: page.page_number,
                page_id: page.page_id,
                contents_id: page.contents_id,
                ast: page.ast.clone(),
                paragraphs: page.paragraphs.clone(),
                metrics: page.metrics.clone(),
            };
        }

        Ok(())
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
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract page images: {}", e)))?;

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
        let obj_id = ObjectId::new(object_id_num);
        let (bytes, mime) = pdf_engine_core::images::get_image_binary(&mut self.doc, obj_id)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to retrieve image binary: {}", e)))?;
        Ok((bytes, mime.to_string()))
    }

    /// Surgically replaces an existing Image XObject in the document with new JPEG or PNG bytes.
    pub fn replace_image(&mut self, object_id_num: u32, new_bytes: &[u8]) -> PyResult<()> {
        let obj_id = ObjectId::new(object_id_num);
        pdf_engine_core::images::replace_image_content(&mut self.doc, obj_id, new_bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to replace image: {}", e)))
    }

    /// Extracts all interactive AcroForm fields from the document.
    pub fn get_form_fields(&mut self) -> PyResult<Vec<PyFormField>> {
        let fields = pdf_engine_core::forms::extract_document_forms(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract form fields: {}", e)))?;

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

    /// Fills a single form field by name or ID.
    pub fn fill_form_field(&mut self, name_or_id: &str, value: &str) -> PyResult<bool> {
        pdf_engine_core::forms::fill_field_value(&mut self.doc, name_or_id, value)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to fill form field: {}", e)))
    }

    /// Fills multiple form fields in a single batch pass.
    pub fn fill_form_fields(
        &mut self,
        values: std::collections::HashMap<String, String>,
    ) -> PyResult<usize> {
        pdf_engine_core::forms::fill_fields_batch(&mut self.doc, &values)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to batch fill form fields: {}", e)))
    }

    /// Flattens all interactive form fields into permanent page content and strips widget annotations.
    pub fn flatten_forms(&mut self) -> PyResult<usize> {
        let count = pdf_engine_core::forms::flatten_document_forms(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to flatten form fields: {}", e)))?;

        self.reload_active_pages();
        Ok(count)
    }

    /// Extracts all non-widget annotations from a specific page.
    pub fn get_page_annotations(&mut self, page_index: usize) -> PyResult<Vec<PyAnnotation>> {
        let zero_idx = if page_index > 0 && page_index <= self.page_ids.len() {
            page_index - 1
        } else {
            page_index
        };
        let annots = pdf_engine_core::annots::extract_page_annotations(&mut self.doc, zero_idx)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract page annotations: {}", e)))?;

        Ok(annots.into_iter().map(PyAnnotation::from_core).collect())
    }

    /// Extracts all non-widget annotations across the entire document.
    pub fn get_all_annotations(&mut self) -> PyResult<Vec<PyAnnotation>> {
        let annots = pdf_engine_core::annots::extract_all_annotations(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to extract document annotations: {}", e)))?;

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
        let annot_id = pdf_engine_core::annots::add_link_goto(
            &mut self.doc,
            zero_idx,
            rect,
            target_zero,
        )
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

    /// Deletes an annotation from a page.
    pub fn delete_annotation(&mut self, page_index: usize, annot_id: u32) -> PyResult<bool> {
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
        let target_idx = page_index.map(|idx| {
            if idx > 0 && idx <= self.page_ids.len() {
                idx - 1
            } else {
                idx
            }
        });
        let count = pdf_engine_core::annots::flatten_annotations(&mut self.doc, target_idx)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to flatten annotations: {}", e)))?;

        self.reload_active_pages();
        Ok(count)
    }

    /// Rotates a page (0-based or 1-based index) by degrees (normalized to 0, 90, 180, 270).
    pub fn rotate_page(&mut self, page_index: usize, degrees: i32) -> PyResult<i32> {
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
        pdf_engine_core::ops::rotate_all_pages(&mut self.doc, degrees)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to rotate all pages: {}", e)))
    }

    /// Extracts specified pages into a new self-contained PyPdfDocument.
    pub fn extract_pages(&mut self, page_indices: Vec<usize>) -> PyResult<Self> {
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
        let mut docs = [self.doc.clone(), other.doc.clone()];
        let merged = pdf_engine_core::ops::merge_documents(&mut docs)
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
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to apply text watermark: {}", e)))?;

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
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to apply image watermark: {}", e)))?;

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

        self.reload_active_pages();
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

        self.reload_active_pages();
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

        self.reload_active_pages();
        Ok(summaries
            .into_iter()
            .map(PyRedactionSummary::from_core)
            .collect())
    }

    /// Scrubs sensitive metadata from the document (/Info dictionary and /Metadata XMP).
    pub fn sanitize_document(&mut self) -> PyResult<bool> {
        let modified = pdf_engine_core::redact::scrub_document_metadata(&mut self.doc)
            .map_err(|e| PyRuntimeError::new_err(format!("Sanitization failed: {}", e)))?;
        Ok(modified)
    }

    /// Encrypts the PDF document using AES-128 standard security handler with user/owner passwords and permissions.
    #[pyo3(signature = (user_password="", owner_password="admin", permissions=None, encrypt_metadata=true))]
    pub fn encrypt(
        &mut self,
        user_password: &str,
        owner_password: &str,
        permissions: Option<PyPdfPermissions>,
        encrypt_metadata: bool,
    ) -> PyResult<()> {
        let perms = permissions.map(|p| p.to_core()).unwrap_or_default();
        let options = EncryptionOptions {
            user_password: user_password.to_string(),
            owner_password: owner_password.to_string(),
            permissions: perms,
            revision: EncryptionRevision::Aes128,
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
        self.reload_active_pages();
        Ok(())
    }

    /// Returns true if the document has an `/Encrypt` trailer dictionary.
    pub fn is_encrypted(&self) -> bool {
        self.doc.xref.trailer.contains_key("Encrypt")
    }

    /// Embeds an ISO 32000-1 §12.8 cryptographic digital signature stamp into the document.
    #[pyo3(signature = (signer_name, reason, location, rect, page_number=1, contact_info=None))]
    pub fn sign(
        &mut self,
        signer_name: &str,
        reason: &str,
        location: &str,
        rect: [f64; 4],
        page_number: usize,
        contact_info: Option<String>,
    ) -> PyResult<PyVerifiedSignature> {
        let config = DigitalSignatureConfig {
            signer_name: signer_name.to_string(),
            reason: reason.to_string(),
            location: location.to_string(),
            rect,
            page_number,
            visual_badge: true,
            contact_info,
        };
        let sig = pdf_engine_core::security::sign_document(&mut self.doc, &config)
            .map_err(|e| PyRuntimeError::new_err(format!("Signing failed: {}", e)))?;
        self.reload_active_pages();
        Ok(PyVerifiedSignature::from_core(sig))
    }

    /// Extracts and cryptographically verifies all embedded digital signatures in the document.
    pub fn verify_signatures(&self) -> PyResult<Vec<PyVerifiedSignature>> {
        let sigs = pdf_engine_core::security::verify_document_signatures(&self.doc);
        Ok(sigs.into_iter().map(PyVerifiedSignature::from_core).collect())
    }

    /// Detects all structured tables on the specified page (1-indexed).
    pub fn extract_tables(&mut self, page_number: usize) -> PyResult<Vec<PyDetectedTable>> {
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

    /// Serializes the modified PDF document to an in-memory byte vector.
    pub fn save_to_bytes(&mut self) -> PyResult<Vec<u8>> {
        self.doc
            .save_to_vec()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to serialize PDF: {}", e)))
    }
}

/// Merges multiple Python PDF documents sequentially into a single unified document.
#[pyfunction]
pub fn merge_documents(docs: Vec<PyRef<'_, PyPdfDocument>>) -> PyResult<PyPdfDocument> {
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

/// The native Python module entrypoint for `pdf_engine`.
#[pymodule]
fn pdf_engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPdfDocument>()?;
    m.add_class::<PyPage>()?;
    m.add_class::<PyParagraph>()?;
    m.add_class::<PyImageInfo>()?;
    m.add_class::<PyFormField>()?;
    m.add_class::<PyAnnotation>()?;
    m.add_class::<PyRedactionSummary>()?;
    m.add_class::<PyPdfPermissions>()?;
    m.add_class::<PyVerifiedSignature>()?;
    m.add_class::<PyTableCell>()?;
    m.add_class::<PyDetectedTable>()?;
    m.add_function(wrap_pyfunction!(merge_documents, m)?)?;
    m.add_function(wrap_pyfunction!(merge_pdf_bytes, m)?)?;
    Ok(())
}
