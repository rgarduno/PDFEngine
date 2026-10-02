//! Python bindings for the PDFEngine core using PyO3.
//!
//! Exposes high-level document loading, page scene graph inspection,
//! and in-place surgical paragraph editing to Python and FastAPI backends.

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::fs;

use pdf_engine_core::cos::{ObjectId, PdfDocument, PdfObject, PdfStream};
use pdf_engine_core::editor::SurgicalEditor;
use pdf_engine_core::fonts::FontMetrics;
use pdf_engine_core::layout::{LayoutReconstructor, ParagraphBlock, TextAlignment};
use pdf_engine_core::stream::{
    build_ast_from_operations, serialize_ast, ContentAst, ContentStreamTokenizer,
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

        // Reload active pages so that get_page() reflects the newly flattened visual operations
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
    m.add_function(wrap_pyfunction!(merge_documents, m)?)?;
    m.add_function(wrap_pyfunction!(merge_pdf_bytes, m)?)?;
    Ok(())
}
