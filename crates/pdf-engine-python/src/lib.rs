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
        let mut doc = PdfDocument::load(bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("PDF parse error: {}", e)))?;

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

/// The native Python module entrypoint for `pdf_engine`.
#[pymodule]
fn pdf_engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPdfDocument>()?;
    m.add_class::<PyPage>()?;
    m.add_class::<PyParagraph>()?;
    Ok(())
}
