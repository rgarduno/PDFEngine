//! Error definitions for the PDF Engine core.
//!
//! Provides typed, structured error representations for lexical, syntactic,
//! decompression, security, and typographic validation failures according to
//! ISO 32000-1.

use thiserror::Error;

/// Result alias for operations within the PDF Engine core.
pub type PdfResult<T> = Result<T, PdfError>;

/// Primary error enum covering all failure modes within the PDF Engine.
#[derive(Debug, Error)]
pub enum PdfError {
    /// Input buffer reached premature EOF during lexical analysis.
    #[error("Unexpected end of file at offset {offset}: {context}")]
    UnexpectedEof {
        offset: usize,
        context: &'static str,
    },

    /// Invalid or unrecognized token encountered during lexical scanning.
    #[error("Lexer error at offset {offset}: {message}")]
    LexerError {
        offset: usize,
        message: String,
    },

    /// Syntax error when parsing PDF objects (arrays, dictionaries, indirect objects).
    #[error("Parse error at offset {offset}: {message}")]
    ParseError {
        offset: usize,
        message: String,
    },

    /// Cross-reference table or cross-reference stream is malformed or corrupted.
    #[error("Invalid cross-reference table/stream at offset {offset}: {message}")]
    InvalidXRef {
        offset: usize,
        message: String,
    },

    /// An indirect object reference could not be resolved.
    #[error("Object not found: {id} gen {gen}")]
    ObjectNotFound {
        id: u32,
        gen: u16,
    },

    /// Object type mismatch when retrieving an expected dictionary, array, or stream.
    #[error("Type mismatch for object {id} gen {gen}: expected {expected}, found {found}")]
    TypeMismatch {
        id: u32,
        gen: u16,
        expected: &'static str,
        found: &'static str,
    },

    /// Stream decompression failure (e.g. invalid zlib header, corrupted Flate stream).
    #[error("Decompression failed for filter '{filter}': {message}")]
    DecompressionError {
        filter: String,
        message: String,
    },

    /// Security policy violation, such as exceeding maximum allowed expansion ratio (Zip Bomb guard).
    #[error("Security policy violation: {0}")]
    SecurityLimitExceeded(String),

    /// Circular reference cycle detected when resolving indirect objects.
    #[error("Circular reference loop detected involving object {id} gen {gen}")]
    CircularReference {
        id: u32,
        gen: u16,
    },

    /// Exceeded maximum recursion depth while traversing nested dictionaries or arrays.
    #[error("Maximum recursion depth ({max_depth}) exceeded at object {id} gen {gen}")]
    RecursionLimitExceeded {
        id: u32,
        gen: u16,
        max_depth: usize,
    },

    /// Font parsing or subsetting failure.
    #[error("Font error for font '{font_name}': {message}")]
    FontError {
        font_name: String,
        message: String,
    },

    /// Content stream operator parsing or execution error.
    #[error("Content stream error: {0}")]
    ContentStreamError(String),

    /// Semantic layout reconstruction error.
    #[error("Layout error: {0}")]
    LayoutError(String),

    /// Invalid page number requested for document.
    #[error("Invalid page index {page} (document has {total} pages)")]
    InvalidPageNumber {
        page: usize,
        total: usize,
    },

    /// Document assembly or manipulation operation error.
    #[error("Document operation error: {0}")]
    OperationError(String),

    /// I/O error encountered while reading or writing PDF data.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
