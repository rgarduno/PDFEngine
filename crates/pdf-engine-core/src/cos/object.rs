//! Carousel Object System (COS) data model according to ISO 32000-1 §7.3.
//!
//! Models all primitive and composite data structures in PDF documents:
//! null, booleans, numeric integers/reals, names, strings, arrays, dictionaries,
//! streams, and indirect object references.

use std::collections::BTreeMap;
use std::fmt;

/// Unique identifier for an indirect object (Object Number, Generation Number).
/// ISO 32000-1 §7.3.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    /// Positive object number assigned in the cross-reference table.
    pub number: u32,
    /// Generation number (usually 0 for non-updated objects, up to 65535).
    pub generation: u16,
}

impl ObjectId {
    /// Creates a new object identifier with generation 0.
    pub const fn new(number: u32) -> Self {
        Self {
            number,
            generation: 0,
        }
    }

    /// Creates an object identifier with specific number and generation.
    pub const fn with_generation(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} R", self.number, self.generation)
    }
}

/// Represents a PDF Name object (ISO 32000-1 §7.3.5).
/// Names in PDF are atomic symbols beginning with a forward slash in syntax (e.g., `/Type`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PdfName(pub String);

impl PdfName {
    /// Creates a new `PdfName` from a string slice without the leading slash.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the raw name value as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PdfName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}", self.0)
    }
}

impl From<&str> for PdfName {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Format of a PDF string literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFormat {
    /// Literal string enclosed in parentheses `(...)`.
    Literal,
    /// Hexadecimal string enclosed in angle brackets `<...>`.
    Hexadecimal,
}

/// Represents a PDF String object (ISO 32000-1 §7.3.4).
/// Can contain raw binary bytes, PDFDocEncoding text, or UTF-16BE text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfString {
    /// The raw byte sequence of the string.
    pub bytes: Vec<u8>,
    /// The syntax format (literal or hexadecimal) used during serialization.
    pub format: StringFormat,
}

impl PdfString {
    /// Creates a literal PDF string from bytes.
    pub fn literal(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
            format: StringFormat::Literal,
        }
    }

    /// Creates a hexadecimal PDF string from bytes.
    pub fn hex(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
            format: StringFormat::Hexadecimal,
        }
    }

    /// Attempts to decode the string bytes into a UTF-8 string.
    /// Handles UTF-16BE BOM (`\xFE\xFF`) and falls back to lossy Latin-1/PDFDocEncoding.
    pub fn to_string_lossy(&self) -> String {
        if self.bytes.starts_with(&[0xFE, 0xFF]) {
            // UTF-16BE encoded string
            let u16_units: Vec<u16> = self.bytes[2..]
                .chunks_exact(2)
                .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                .collect();
            String::from_utf16_lossy(&u16_units)
        } else {
            // Treat as ISO-8859-1 / ASCII fallback
            self.bytes.iter().map(|&b| b as char).collect()
        }
    }
}

/// Represents a PDF Dictionary (ISO 32000-1 §7.3.7).
/// An associative table mapping `PdfName` keys to `PdfObject` values.
/// Preserves deterministic key ordering using `BTreeMap`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PdfDictionary(pub BTreeMap<PdfName, PdfObject>);

impl PdfDictionary {
    /// Creates a new empty dictionary.
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// Inserts a key-value pair into the dictionary.
    pub fn insert(&mut self, key: impl Into<PdfName>, value: impl Into<PdfObject>) {
        self.0.insert(key.into(), value.into());
    }

    /// Retrieves a reference to an object by key name.
    pub fn get(&self, key: &str) -> Option<&PdfObject> {
        self.0.get(&PdfName(key.to_string()))
    }

    /// Retrieves a mutable reference to an object by key name.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut PdfObject> {
        self.0.get_mut(&PdfName(key.to_string()))
    }

    /// Removes a key from the dictionary.
    pub fn remove(&mut self, key: &str) -> Option<PdfObject> {
        self.0.remove(&PdfName(key.to_string()))
    }

    /// Returns the number of entries in the dictionary.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the dictionary contains no entries.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Checks whether the dictionary contains a specific key.
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(&PdfName(key.to_string()))
    }
}

/// Type alias for PDF Array objects (ISO 32000-1 §7.3.6).
pub type PdfArray = Vec<PdfObject>;

/// Represents a PDF Stream object (ISO 32000-1 §7.3.8).
/// A stream consists of a dictionary describing its metadata (length, filters)
/// followed by a sequence of raw or compressed bytes delimited by `stream ... endstream`.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfStream {
    /// Dictionary describing stream attributes (`/Length`, `/Filter`, `/DecodeParms`, etc.).
    pub dict: PdfDictionary,
    /// Raw byte contents of the stream.
    pub content: Vec<u8>,
}

impl PdfStream {
    /// Constructs a new stream from metadata dictionary and raw content.
    pub fn new(dict: PdfDictionary, content: Vec<u8>) -> Self {
        Self { dict, content }
    }
}

/// The core enumeration representing any valid COS object in a PDF document.
/// ISO 32000-1 §7.3.
#[derive(Debug, Clone, PartialEq)]
pub enum PdfObject {
    /// The `null` object (§7.3.9).
    Null,
    /// Boolean values `true` or `false` (§7.3.2).
    Boolean(bool),
    /// 64-bit signed integer value (§7.3.3).
    Integer(i64),
    /// 64-bit IEEE-754 floating point real number (§7.3.3).
    Real(f64),
    /// Name token (§7.3.5).
    Name(PdfName),
    /// String literal or hexadecimal sequence (§7.3.4).
    String(PdfString),
    /// Ordered sequence of objects enclosed in `[...]` (§7.3.6).
    Array(Vec<PdfObject>),
    /// Associative dictionary enclosed in `<< ... >>` (§7.3.7).
    Dictionary(PdfDictionary),
    /// Data stream containing dictionary metadata and byte payload (§7.3.8).
    Stream(PdfStream),
    /// Indirect object reference `n g R` (§7.3.10).
    Reference(ObjectId),
}

impl PdfObject {
    /// Returns true if this object is a null token.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Attempts to extract an `i64` integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Attempts to extract an `f64` numeric value (converting integers if necessary).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Real(r) => Some(*r),
            Self::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Attempts to extract a string reference.
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(n) => Some(n.as_str()),
            _ => None,
        }
    }

    /// Attempts to extract a reference to an underlying dictionary.
    pub fn as_dict(&self) -> Option<&PdfDictionary> {
        match self {
            Self::Dictionary(d) => Some(d),
            Self::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    /// Attempts to extract a mutable reference to an underlying dictionary.
    pub fn as_dict_mut(&mut self) -> Option<&mut PdfDictionary> {
        match self {
            Self::Dictionary(d) => Some(d),
            Self::Stream(s) => Some(&mut s.dict),
            _ => None,
        }
    }

    /// Attempts to extract a reference to an array.
    pub fn as_array(&self) -> Option<&[PdfObject]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Attempts to extract a mutable reference to an array.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<PdfObject>> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Attempts to extract a PdfString.
    pub fn as_string(&self) -> Option<&PdfString> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Attempts to extract the raw byte slice of a string or name.
    pub fn as_string_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(s) => Some(&s.bytes),
            Self::Name(n) => Some(n.0.as_bytes()),
            _ => None,
        }
    }

    /// Attempts to extract a reference to an indirect object id.
    pub fn as_reference(&self) -> Option<ObjectId> {
        match self {
            Self::Reference(id) => Some(*id),
            _ => None,
        }
    }
}

impl From<bool> for PdfObject {
    fn from(b: bool) -> Self {
        Self::Boolean(b)
    }
}

impl From<i64> for PdfObject {
    fn from(i: i64) -> Self {
        Self::Integer(i)
    }
}

impl From<f64> for PdfObject {
    fn from(f: f64) -> Self {
        Self::Real(f)
    }
}

impl From<PdfName> for PdfObject {
    fn from(n: PdfName) -> Self {
        Self::Name(n)
    }
}

impl From<PdfString> for PdfObject {
    fn from(s: PdfString) -> Self {
        Self::String(s)
    }
}

impl From<PdfDictionary> for PdfObject {
    fn from(d: PdfDictionary) -> Self {
        Self::Dictionary(d)
    }
}

impl From<Vec<PdfObject>> for PdfObject {
    fn from(a: Vec<PdfObject>) -> Self {
        Self::Array(a)
    }
}
