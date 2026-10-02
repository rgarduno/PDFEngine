//! Carousel Object System (COS) implementation according to ISO 32000-1.
//!
//! Exposes low-level data structures, lexer, parser, cross-reference indexer,
//! decompression filters, and high-level `PdfDocument` container.

pub mod filters;
pub mod lexer;
pub mod object;
pub mod parser;
pub mod writer;
pub mod xref;

use std::collections::{BTreeMap, HashSet};

pub use filters::decode_stream;
pub use lexer::{Lexer, Token};
pub use object::{ObjectId, PdfArray, PdfDictionary, PdfName, PdfObject, PdfStream, PdfString, StringFormat};
pub use parser::Parser;
pub use writer::Writer;
pub use xref::{XRefEntry, XRefTable};

use crate::error::{PdfError, PdfResult};
use crate::security::SecurityLimits;

/// Represents an in-memory parsed PDF document container.
///
/// Thread-safe, stateless, and suitable for high-concurrency cloud environments
/// and serverless workers.
#[derive(Debug, Clone)]
pub struct PdfDocument {
    /// Raw unparsed byte buffer of the loaded PDF.
    raw_data: Vec<u8>,
    /// Cross-reference index resolved from file trailers and xref tables.
    pub xref: XRefTable,
    /// In-memory cache of modified or newly created indirect objects.
    pub objects: BTreeMap<ObjectId, PdfObject>,
    /// Security limits active for this document.
    pub limits: SecurityLimits,
}

impl PdfDocument {
    /// Loads and parses a PDF document from a byte buffer.
    ///
    /// # Arguments
    /// * `data` - Complete byte slice of the PDF file.
    pub fn load(data: &[u8]) -> PdfResult<Self> {
        Self::load_with_limits(data, SecurityLimits::default())
    }

    /// Loads and parses a PDF document enforcing custom security limits.
    pub fn load_with_limits(data: &[u8], limits: SecurityLimits) -> PdfResult<Self> {
        if data.len() < 8 {
            return Err(PdfError::UnexpectedEof {
                offset: 0,
                context: "File too small to be a valid PDF document",
            });
        }

        // Verify PDF magic header `%PDF-`
        if !data.starts_with(b"%PDF-") {
            return Err(PdfError::ParseError {
                offset: 0,
                message: "Missing '%PDF-' header signature".to_string(),
            });
        }

        // Locate startxref offset from tail
        let startxref_offset = XRefTable::find_startxref(data)?;
        let xref = XRefTable::load_chain(data, startxref_offset, &limits)?;

        Ok(Self {
            raw_data: data.to_vec(),
            xref,
            objects: BTreeMap::new(),
            limits,
        })
    }

    /// Creates an empty valid ISO 32000-1 PDF document with a clean Catalog and Pages root.
    pub fn empty() -> Self {
        let mut doc = Self {
            raw_data: Vec::new(),
            xref: XRefTable::new(),
            objects: BTreeMap::new(),
            limits: SecurityLimits::default(),
        };

        let catalog_id = ObjectId::new(1);
        let pages_id = ObjectId::new(2);

        let mut pages_dict = PdfDictionary::new();
        pages_dict.insert("Type", PdfName::new("Pages"));
        pages_dict.insert("Kids", PdfArray::new());
        pages_dict.insert("Count", 0i64);
        doc.set_object(pages_id, PdfObject::Dictionary(pages_dict));

        let mut catalog_dict = PdfDictionary::new();
        catalog_dict.insert("Type", PdfName::new("Catalog"));
        catalog_dict.insert("Pages", pages_id);
        doc.set_object(catalog_id, PdfObject::Dictionary(catalog_dict));

        doc.xref.trailer.insert("Root", catalog_id);
        doc.xref.trailer.insert("Size", 3i64);

        doc
    }

    /// Returns the object ID of the `/Catalog` root dictionary from the trailer.
    pub fn catalog_id(&self) -> Option<ObjectId> {
        self.xref.trailer.get("Root").and_then(|r| r.as_reference())
    }

    /// Returns the object ID of the root `/Pages` dictionary.
    pub fn pages_id(&mut self) -> PdfResult<ObjectId> {
        let catalog = self.catalog()?;
        catalog
            .get("Pages")
            .and_then(|p| p.as_reference())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "Catalog missing /Pages reference".to_string(),
            })
    }

    /// Resolves an indirect object by identifier, parsing from buffer if not yet cached.
    pub fn get_object(&mut self, id: ObjectId) -> PdfResult<PdfObject> {
        if let Some(obj) = self.objects.get(&id) {
            return Ok(obj.clone());
        }

        let entry = self
            .xref
            .get(id)
            .copied()
            .ok_or(PdfError::ObjectNotFound {
                id: id.number,
                gen: id.generation,
            })?;

        match entry {
            XRefEntry::InUse { offset, .. } => {
                let mut parser = Parser::at_offset(&self.raw_data, offset as usize);
                let (parsed_id, obj) = parser.parse_indirect_object()?;
                if parsed_id.number != id.number {
                    return Err(PdfError::InvalidXRef {
                        offset: offset as usize,
                        message: format!(
                            "XRef pointed to object {}, but found object {}",
                            id, parsed_id
                        ),
                    });
                }
                self.objects.insert(id, obj.clone());
                Ok(obj)
            }
            XRefEntry::Compressed {
                container_id,
                index_in_stream,
            } => {
                // Object stored in an Object Stream (/ObjStm)
                let container_obj = self.get_object(ObjectId::new(container_id))?;
                if let PdfObject::Stream(stream) = container_obj {
                    let obj = self.extract_from_obj_stm(&stream, index_in_stream)?;
                    self.objects.insert(id, obj.clone());
                    Ok(obj)
                } else {
                    Err(PdfError::TypeMismatch {
                        id: container_id,
                        gen: 0,
                        expected: "Stream",
                        found: "Non-stream object",
                    })
                }
            }
            XRefEntry::Free { .. } => Err(PdfError::ObjectNotFound {
                id: id.number,
                gen: id.generation,
            }),
        }
    }

    /// Extracts an object from an `/ObjStm` compressed stream (ISO 32000-1 §7.5.7).
    fn extract_from_obj_stm(
        &self,
        stream: &PdfStream,
        target_index: u16,
    ) -> PdfResult<PdfObject> {
        let filter_name = stream
            .dict
            .get("Filter")
            .and_then(|f| f.as_name())
            .unwrap_or("FlateDecode");
        let decode_parms = stream.dict.get("DecodeParms").and_then(|p| p.as_dict());

        let decompressed = decode_stream(filter_name, decode_parms, &stream.content, &self.limits)?;

        let first_offset = stream
            .dict
            .get("First")
            .and_then(|f| f.as_i64())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "ObjStm missing /First integer".to_string(),
            })? as usize;

        let num_objects = stream
            .dict
            .get("N")
            .and_then(|n| n.as_i64())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "ObjStm missing /N integer".to_string(),
            })? as usize;

        if target_index as usize >= num_objects {
            return Err(PdfError::InvalidXRef {
                offset: 0,
                message: format!(
                    "ObjStm index {} out of bounds (N = {})",
                    target_index, num_objects
                ),
            });
        }

        // Header consists of N pairs of [obj_num relative_offset]
        let mut lexer = Lexer::new(&decompressed[..first_offset]);
        let mut offsets = Vec::with_capacity(num_objects);

        for _ in 0..num_objects {
            let _obj_id = match lexer.next_token()? {
                Some(Token::Integer(id)) => id as u32,
                _ => break,
            };
            let offset = match lexer.next_token()? {
                Some(Token::Integer(off)) => off as usize,
                _ => break,
            };
            offsets.push(offset);
        }

        let rel_offset = offsets
            .get(target_index as usize)
            .copied()
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: format!("ObjStm relative offset not found for index {}", target_index),
            })?;

        let obj_start = first_offset + rel_offset;
        let mut parser = Parser::at_offset(&decompressed, obj_start);
        parser.parse_object()
    }

    /// Retrieves the document `/Catalog` root dictionary (ISO 32000-1 §7.7.2).
    pub fn catalog(&mut self) -> PdfResult<PdfDictionary> {
        let root_ref = self
            .xref
            .trailer
            .get("Root")
            .and_then(|r| r.as_reference())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "Document trailer missing mandatory /Root reference".to_string(),
            })?;

        let root_obj = self.get_object(root_ref)?;
        match root_obj {
            PdfObject::Dictionary(d) => Ok(d),
            _ => Err(PdfError::TypeMismatch {
                id: root_ref.number,
                gen: root_ref.generation,
                expected: "Dictionary",
                found: "Non-dictionary root",
            }),
        }
    }

    /// Collects all page object identifiers in reading order traversing the `/Pages` tree.
    pub fn get_pages(&mut self) -> PdfResult<Vec<ObjectId>> {
        let catalog = self.catalog()?;
        let pages_ref = catalog
            .get("Pages")
            .and_then(|p| p.as_reference())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "Catalog missing /Pages reference".to_string(),
            })?;

        let mut pages = Vec::new();
        let mut visited = HashSet::new();
        self.traverse_pages_node(pages_ref, &mut pages, &mut visited, 0)?;
        Ok(pages)
    }

    /// Internal recursive page tree traversal with cycle detection.
    fn traverse_pages_node(
        &mut self,
        node_id: ObjectId,
        pages: &mut Vec<ObjectId>,
        visited: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> PdfResult<()> {
        if depth > self.limits.max_recursion_depth {
            return Err(PdfError::RecursionLimitExceeded {
                id: node_id.number,
                gen: node_id.generation,
                max_depth: self.limits.max_recursion_depth,
            });
        }

        if !visited.insert(node_id) {
            return Err(PdfError::CircularReference {
                id: node_id.number,
                gen: node_id.generation,
            });
        }

        let obj = self.get_object(node_id)?;
        let dict = match obj {
            PdfObject::Dictionary(d) => d,
            _ => return Ok(()),
        };

        let type_name = dict.get("Type").and_then(|t| t.as_name()).unwrap_or("");

        if type_name == "Page" {
            pages.push(node_id);
        } else if type_name == "Pages" {
            if let Some(kids) = dict.get("Kids").and_then(|k| k.as_array()) {
                let kid_refs: Vec<ObjectId> = kids.iter().filter_map(|k| k.as_reference()).collect();
                for kid in kid_refs {
                    self.traverse_pages_node(kid, pages, visited, depth + 1)?;
                }
            }
        }

        Ok(())
    }

    /// Inserts or replaces an object in the document cache and xref index.
    pub fn set_object(&mut self, id: ObjectId, obj: PdfObject) {
        self.objects.insert(id, obj);
        self.xref.entries.insert(
            id,
            XRefEntry::InUse {
                offset: 0,
                generation: id.generation,
            },
        );
    }

    /// Allocates a new unused `ObjectId`.
    pub fn alloc_object_id(&self) -> ObjectId {
        let max_from_objects = self.objects.keys().map(|id| id.number).max().unwrap_or(0);
        let max_from_xref = self.xref.entries.keys().map(|id| id.number).max().unwrap_or(0);
        ObjectId::new(max_from_objects.max(max_from_xref) + 1)
    }

    /// Retrieves a mutable reference to a stream object.
    pub fn get_stream_mut(&mut self, id: ObjectId) -> PdfResult<&mut PdfStream> {
        let _ = self.get_object(id)?;
        match self.objects.get_mut(&id) {
            Some(PdfObject::Stream(s)) => Ok(s),
            Some(_) => Err(PdfError::TypeMismatch {
                id: id.number,
                gen: id.generation,
                expected: "Stream",
                found: "Non-stream object",
            }),
            None => Err(PdfError::ObjectNotFound {
                id: id.number,
                gen: id.generation,
            }),
        }
    }

    /// Extracts decompressed page content stream bytes.
    pub fn get_page_content_bytes(&mut self, page_id: ObjectId) -> PdfResult<Vec<u8>> {
        let page_obj = self.get_object(page_id)?;
        let page_dict = match page_obj {
            PdfObject::Dictionary(d) => d,
            _ => return Ok(Vec::new()),
        };

        match page_dict.get("Contents") {
            Some(PdfObject::Reference(r)) => {
                let stream_obj = self.get_object(*r)?;
                if let PdfObject::Stream(s) = stream_obj {
                    let filter = s.dict.get("Filter").and_then(|f| f.as_name()).unwrap_or("");
                    let decode_parms = s.dict.get("DecodeParms").and_then(|p| p.as_dict());
                    decode_stream(filter, decode_parms, &s.content, &self.limits)
                } else {
                    Ok(Vec::new())
                }
            }
            Some(PdfObject::Array(arr)) => {
                let mut combined = Vec::new();
                for item in arr.iter() {
                    if let Some(r) = item.as_reference() {
                        let stream_obj = self.get_object(r)?;
                        if let PdfObject::Stream(s) = stream_obj {
                            let filter = s.dict.get("Filter").and_then(|f| f.as_name()).unwrap_or("");
                            let decode_parms = s.dict.get("DecodeParms").and_then(|p| p.as_dict());
                            let chunk = decode_stream(filter, decode_parms, &s.content, &self.limits)?;
                            combined.extend_from_slice(&chunk);
                            combined.push(b'\n');
                        }
                    }
                }
                Ok(combined)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Extracts embedded font binaries (TrueType / OpenType) from a specific page.
    /// Returns a map of font names (e.g. "F1", "Helvetica") to their raw decompressed font file bytes.
    pub fn extract_page_fonts(
        &mut self,
        page_id: ObjectId,
    ) -> PdfResult<std::collections::HashMap<String, Vec<u8>>> {
        let mut fonts = std::collections::HashMap::new();

        let page_obj = self.get_object(page_id)?;
        let page_dict = match page_obj {
            PdfObject::Dictionary(d) => d,
            _ => return Ok(fonts),
        };

        let resources = match page_dict.get("Resources") {
            Some(PdfObject::Dictionary(d)) => d.clone(),
            Some(PdfObject::Reference(r)) => match self.get_object(*r)? {
                PdfObject::Dictionary(d) => d,
                _ => return Ok(fonts),
            },
            _ => return Ok(fonts),
        };

        let font_dict = match resources.get("Font") {
            Some(PdfObject::Dictionary(d)) => d.clone(),
            Some(PdfObject::Reference(r)) => match self.get_object(*r)? {
                PdfObject::Dictionary(d) => d,
                _ => return Ok(fonts),
            },
            _ => return Ok(fonts),
        };

        for (font_key, font_val) in font_dict.0 {
            let font_obj = match font_val {
                PdfObject::Dictionary(d) => d,
                PdfObject::Reference(r) => match self.get_object(r)? {
                    PdfObject::Dictionary(d) => d,
                    _ => continue,
                },
                _ => continue,
            };

            let descriptor_obj = match font_obj.get("FontDescriptor") {
                Some(PdfObject::Dictionary(d)) => d.clone(),
                Some(PdfObject::Reference(r)) => match self.get_object(*r)? {
                    PdfObject::Dictionary(d) => d,
                    _ => continue,
                },
                _ => continue,
            };

            // Check /FontFile2 (TrueType) or /FontFile3 (CFF/OpenType)
            let font_file_ref = descriptor_obj
                .get("FontFile2")
                .or_else(|| descriptor_obj.get("FontFile3"))
                .and_then(|f| f.as_reference());

            if let Some(stream_ref) = font_file_ref {
                if let PdfObject::Stream(s) = self.get_object(stream_ref)? {
                    let filter = s.dict.get("Filter").and_then(|f| f.as_name()).unwrap_or("");
                    let decode_parms = s.dict.get("DecodeParms").and_then(|p| p.as_dict());
                    let font_bytes = decode_stream(filter, decode_parms, &s.content, &self.limits)?;
                    fonts.insert(font_key.as_str().to_string(), font_bytes);
                }
            }
        }

        Ok(fonts)
    }

    /// Serializes the complete document to a byte vector.
    pub fn save_to_vec(&mut self) -> PdfResult<Vec<u8>> {
        let mut out = Vec::new();
        let mut writer = Writer::new(&mut out);

        writer.write_header("1.7")?;

        let mut offsets = Vec::new();

        // Collect all in-use object IDs from xref
        let all_ids: Vec<ObjectId> = self
            .xref
            .entries
            .iter()
            .filter_map(|(&id, entry)| match entry {
                XRefEntry::InUse { .. } | XRefEntry::Compressed { .. } => Some(id),
                XRefEntry::Free { .. } => None,
            })
            .collect();

        // Ensure all in-use objects are loaded into cache
        for id in all_ids {
            let _ = self.get_object(id);
        }

        // Write all objects present in the objects map sorted by number
        let mut sorted_keys: Vec<ObjectId> = self.objects.keys().copied().collect();
        sorted_keys.sort_by_key(|id| id.number);

        for id in sorted_keys {
            if let Some(obj) = self.objects.get(&id) {
                let offset = writer.write_indirect_object(id, obj)?;
                offsets.push((id, offset));
            }
        }

        let mut trailer = self.xref.trailer.clone();
        trailer.insert("Size", (offsets.len() + 1) as i64);
        writer.write_xref_and_trailer(&offsets, &trailer)?;

        Ok(out)
    }
}
