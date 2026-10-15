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

pub use filters::{decode_stream, encode_flate, Compression};
pub use lexer::{Lexer, Token};
pub use object::{
    ObjectId, PdfArray, PdfDictionary, PdfName, PdfObject, PdfStream, PdfString, StringFormat,
};
pub use parser::Parser;
pub use writer::Writer;
pub use xref::{XRefEntry, XRefTable};

use crate::error::{PdfError, PdfResult};
use crate::security::permissions::{PdfPermissions, PERMISSION_DENIED};
use crate::security::SecurityLimits;

/// In-memory parsed PDF document.
///
/// The value owns its file bytes and object map. It keeps no process-global
/// state and is `Send`, so a caller may move it onto another thread. Mutating
/// one document from several threads still requires the caller to synchronize.
/// An API process that keeps documents for later requests has its own session
/// table; this type does not.
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
    /// Header version emitted by [`Self::save_to_vec`].
    ///
    /// Parsing always stores `1.7`. PDF/A-1b conversion sets `1.4` on the
    /// document it is about to save; the bytes, not this field, are what a
    /// later load can prove.
    pub write_version: String,
    /// Cross-reference entries captured at load. [`Self::set_object`] stores a
    /// live offset of 0, so an incremental save reparses the signed bytes from
    /// this snapshot.
    loaded_entries: BTreeMap<ObjectId, XRefEntry>,
    /// Trailer dictionary captured at load, before later edits.
    loaded_trailer: PdfDictionary,
    /// Bits remembered after a user password opens the file.
    ///
    /// `None` means this session is unrestricted: the file was not opened with a
    /// user password, the owner password was used, or every enforced bit was set.
    /// Bit 10 and bit 12 are not enforced bits. A restricted value blocks later
    /// print, copy, and edit in this process. Saving the decrypted file drops `/P`.
    user_permissions: Option<PdfPermissions>,
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
        let loaded_entries = xref.entries.clone();
        let loaded_trailer = xref.trailer.clone();

        Ok(Self {
            raw_data: data.to_vec(),
            xref,
            objects: BTreeMap::new(),
            limits,
            write_version: "1.7".to_string(),
            loaded_entries,
            loaded_trailer,
            user_permissions: None,
        })
    }

    /// Returns the raw unparsed byte buffer of the loaded PDF.
    pub fn raw_data(&self) -> &[u8] {
        &self.raw_data
    }

    /// Drops the retained source file after its objects have been cached.
    ///
    /// The object map and cross-reference table stay. Call this only after
    /// every object the next step needs has been loaded: an uncached object is
    /// parsed from these bytes.
    pub(crate) fn release_retained_file(&mut self) {
        self.raw_data = Vec::new();
    }

    /// Creates an empty valid ISO 32000-1 PDF document with a clean Catalog and Pages root.
    pub fn empty() -> Self {
        let mut doc = Self {
            raw_data: Vec::new(),
            xref: XRefTable::new(),
            objects: BTreeMap::new(),
            limits: SecurityLimits::default(),
            write_version: "1.7".to_string(),
            loaded_entries: BTreeMap::new(),
            loaded_trailer: PdfDictionary::new(),
            user_permissions: None,
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
        self.resolve_object(id, &mut HashSet::new(), 0)
    }

    /// Resolves `id`, rejecting a cycle or a walk deeper than the configured limit.
    ///
    /// A cached object returns before it is recorded on `stack`: the cache is the
    /// object itself, not another lookup. `stack` is the chain of objects whose
    /// bodies are still being read.
    fn resolve_object(
        &mut self,
        id: ObjectId,
        stack: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> PdfResult<PdfObject> {
        if let Some(obj) = self.objects.get(&id) {
            return Ok(obj.clone());
        }

        self.limits
            .validate_depth(depth, id.number, id.generation)?;
        if !stack.insert(id) {
            return Err(PdfError::CircularReference {
                id: id.number,
                gen: id.generation,
            });
        }

        let result = self.read_uncached_object(id, stack, depth);
        stack.remove(&id);
        result
    }

    /// Parses an object that is not in the cache and stores it when the count allows.
    fn read_uncached_object(
        &mut self,
        id: ObjectId,
        stack: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> PdfResult<PdfObject> {
        let entry = self.xref.get(id).copied().ok_or(PdfError::ObjectNotFound {
            id: id.number,
            gen: id.generation,
        })?;

        match entry {
            XRefEntry::InUse { offset, .. } => {
                let parsed = {
                    let mut parser = Parser::at_offset(&self.raw_data, offset as usize);
                    parser.parse_indirect_object()?
                };
                let (parsed_id, obj) = parsed;
                if parsed_id.number != id.number {
                    return Err(PdfError::InvalidXRef {
                        offset: offset as usize,
                        message: format!(
                            "XRef pointed to object {}, but found object {}",
                            id, parsed_id
                        ),
                    });
                }
                self.remember_object(id, obj)
            }
            XRefEntry::Compressed {
                container_id,
                index_in_stream,
            } => {
                let container_obj =
                    self.resolve_object(ObjectId::new(container_id), stack, depth + 1)?;
                let PdfObject::Stream(stream) = container_obj else {
                    return Err(PdfError::TypeMismatch {
                        id: container_id,
                        gen: 0,
                        expected: "Stream",
                        found: "Non-stream object",
                    });
                };
                let obj = self.extract_from_obj_stm(&stream, index_in_stream)?;
                self.remember_object(id, obj)
            }
            XRefEntry::Free { .. } => Err(PdfError::ObjectNotFound {
                id: id.number,
                gen: id.generation,
            }),
        }
    }

    /// Inserts a newly parsed object when the cache is still within `max_object_count`.
    fn remember_object(&mut self, id: ObjectId, obj: PdfObject) -> PdfResult<PdfObject> {
        self.limits
            .validate_object_count(self.objects.len().saturating_add(1))?;
        self.objects.insert(id, obj.clone());
        Ok(obj)
    }

    /// Extracts an object from an `/ObjStm` compressed stream (ISO 32000-1 §7.5.7).
    fn extract_from_obj_stm(&self, stream: &PdfStream, target_index: u16) -> PdfResult<PdfObject> {
        let declared = stream
            .dict
            .get("N")
            .and_then(|n| n.as_i64())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "ObjStm missing /N integer".to_string(),
            })?;
        // A negative `/N` must not be cast to `usize` (that becomes a huge capacity).
        if declared < 0 || (declared as u64) > self.limits.max_object_count as u64 {
            return Err(PdfError::SecurityLimitExceeded(format!(
                "ObjStm /N ({}) is outside the allowed object count",
                declared
            )));
        }
        let num_objects = declared as usize;

        let filter_name = stream
            .dict
            .get("Filter")
            .and_then(|f| f.as_name())
            .unwrap_or("FlateDecode");
        let decode_parms = stream.dict.get("DecodeParms").and_then(|p| p.as_dict());

        let decompressed = decode_stream(filter_name, decode_parms, &stream.content, &self.limits)?;

        let first = stream
            .dict
            .get("First")
            .and_then(|f| f.as_i64())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "ObjStm missing /First integer".to_string(),
            })?;
        if first < 0 || (first as usize) > decompressed.len() {
            return Err(PdfError::InvalidXRef {
                offset: 0,
                message: "ObjStm /First is outside the decoded stream".to_string(),
            });
        }
        let first_offset = first as usize;

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

        let rel_offset =
            offsets
                .get(target_index as usize)
                .copied()
                .ok_or_else(|| PdfError::InvalidXRef {
                    offset: 0,
                    message: format!(
                        "ObjStm relative offset not found for index {}",
                        target_index
                    ),
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
                let kid_refs: Vec<ObjectId> =
                    kids.iter().filter_map(|k| k.as_reference()).collect();
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
        let max_from_xref = self
            .xref
            .entries
            .keys()
            .map(|id| id.number)
            .max()
            .unwrap_or(0);
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
                            let filter =
                                s.dict.get("Filter").and_then(|f| f.as_name()).unwrap_or("");
                            let decode_parms = s.dict.get("DecodeParms").and_then(|p| p.as_dict());
                            let chunk =
                                decode_stream(filter, decode_parms, &s.content, &self.limits)?;
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

    /// Serializes the document.
    ///
    /// A file whose stored bytes still verify `/adbe.pkcs7.detached` gains one
    /// incremental update. The signed prefix stays in place, so that byte range
    /// still matches. Bytes in the update are not covered by the same signature.
    /// Any other file is rewritten, and a SHA-256 attestation is sealed again.
    pub fn save_to_vec(&mut self) -> PdfResult<Vec<u8>> {
        if self.detached_pkcs7_covers_source() {
            return self.save_incremental();
        }
        self.save_rewritten()
    }

    /// True when `raw_data` still contains a detached PKCS#7 signature that verifies.
    ///
    /// The check reads the stored bytes. A signature that is about to be written,
    /// and a signature whose byte range no longer matches, stay on the full rewrite.
    fn detached_pkcs7_covers_source(&self) -> bool {
        if self.raw_data.is_empty()
            || !self
                .raw_data
                .windows(crate::security::cms::PKCS7_MARK.len())
                .any(|window| window == crate::security::cms::PKCS7_MARK)
        {
            return false;
        }
        crate::security::signatures::verify_document_signatures(self)
            .iter()
            .any(|sig| sig.sub_filter == "adbe.pkcs7.detached" && sig.byte_range_valid)
    }

    /// Appends objects that differ from the loaded revision, plus a classic xref.
    ///
    /// An unchanged file is returned as stored. `/Encrypt` on a signed file is
    /// refused when there is something to append: plaintext objects must not
    /// follow an encrypted body. Active-content keys are removed from the
    /// appended objects only. The signed prefix is not rewritten.
    fn save_incremental(&self) -> PdfResult<Vec<u8>> {
        let mut changed = Vec::new();
        let mut ids: Vec<ObjectId> = self.objects.keys().copied().collect();
        ids.sort_by_key(|id| (id.number, id.generation));
        for id in ids {
            if id.number == 0 {
                continue;
            }
            let mut stack = HashSet::new();
            let same = match self.source_object(id, &mut stack, 0)? {
                Some(source) => source == self.objects[&id],
                None => false,
            };
            if !same {
                changed.push((id, self.objects[&id].clone()));
            }
        }

        let changed_numbers: HashSet<u32> = changed.iter().map(|(id, _)| id.number).collect();
        let mut freed = Vec::new();
        for (id, entry) in &self.loaded_entries {
            if id.number == 0 || changed_numbers.contains(&id.number) {
                continue;
            }
            if self.xref.entries.contains_key(id) {
                continue;
            }
            let generation = match entry {
                XRefEntry::InUse { generation, .. } => generation.saturating_add(1),
                XRefEntry::Compressed { .. } => 1,
                XRefEntry::Free { .. } => continue,
            };
            freed.push((id.number, generation));
        }
        freed.sort_by_key(|entry| entry.0);

        let trailer_dirty = self.xref.trailer != self.loaded_trailer;
        if changed.is_empty() && freed.is_empty() && !trailer_dirty {
            return Ok(self.raw_data.clone());
        }
        if self.xref.trailer.contains_key("Encrypt") || self.loaded_trailer.contains_key("Encrypt")
        {
            return Err(PdfError::CryptographyError(
                "Incremental save is refused for an encrypted PKCS#7 document.".to_string(),
            ));
        }

        let mut scratch = self.scrub_surface(&changed);
        crate::security::active::neutralize_active_content(&mut scratch)?;

        let mut append = Vec::new();
        for (id, _) in &changed {
            let Some(obj) = scratch.objects.get(id) else {
                continue;
            };
            let mut stack = HashSet::new();
            let same = match self.source_object(*id, &mut stack, 0)? {
                Some(source) => source == *obj,
                None => false,
            };
            if !same {
                append.push((*id, obj.clone()));
            }
        }
        let scrubbed_trailer = scratch.xref.trailer.clone();
        if append.is_empty() && freed.is_empty() && scrubbed_trailer == self.loaded_trailer {
            return Ok(self.raw_data.clone());
        }

        let mut trailer = scrubbed_trailer;
        if trailer.get("Type").and_then(|obj| obj.as_name()) == Some("XRef") {
            trailer.remove("Type");
        }
        for key in ["W", "Index", "Filter", "DecodeParms", "Length", "XRefStm"] {
            trailer.remove(key);
        }
        let prev = XRefTable::find_startxref(&self.raw_data)?;
        trailer.insert("Prev", prev as i64);
        let mut highest = 0u32;
        for id in self.loaded_entries.keys() {
            highest = highest.max(id.number);
        }
        for (id, _) in &append {
            highest = highest.max(id.number);
        }
        for (number, _) in &freed {
            highest = highest.max(*number);
        }
        trailer.insert("Size", i64::from(highest) + 1);

        let mut out = self.raw_data.clone();
        if out.last() != Some(&b'\n') {
            out.push(b'\n');
        }
        let prefix_len = out.len();
        let mut writer = Writer::with_offset(&mut out, prefix_len);
        let mut offsets = Vec::new();
        for (id, obj) in &append {
            let write_id = ObjectId::with_generation(id.number, self.source_generation(*id));
            let offset = writer.write_indirect_object(write_id, obj)?;
            offsets.push((write_id, offset));
        }
        writer.write_xref_and_trailer_with_free(&offsets, &freed, &trailer)?;
        Ok(out)
    }

    /// Document that holds only the objects and trailer about to be appended.
    ///
    /// Its cross-reference has no entries, so neutralization cannot load or
    /// rewrite the signed prefix. `self` stays unchanged when this save fails.
    fn scrub_surface(&self, changed: &[(ObjectId, PdfObject)]) -> Self {
        let mut objects = BTreeMap::new();
        for (id, obj) in changed {
            objects.insert(*id, obj.clone());
        }
        let mut xref = XRefTable::new();
        xref.trailer = self.xref.trailer.clone();
        Self {
            raw_data: Vec::new(),
            xref,
            objects,
            limits: self.limits.clone(),
            write_version: self.write_version.clone(),
            loaded_entries: BTreeMap::new(),
            loaded_trailer: PdfDictionary::new(),
            user_permissions: None,
        }
    }

    /// Generation stored for `id` in the loaded revision. A new object uses 0.
    fn source_generation(&self, id: ObjectId) -> u16 {
        match self.loaded_entries.get(&ObjectId::new(id.number)) {
            Some(XRefEntry::InUse { generation, .. }) => *generation,
            _ => 0,
        }
    }

    /// Parses `id` from the loaded revision. `None` means the object is new.
    ///
    /// The live cross-reference is ignored: a mutation stores offset 0 there.
    /// A compressed object is read from the file container, not from the cache.
    fn source_object(
        &self,
        id: ObjectId,
        stack: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> PdfResult<Option<PdfObject>> {
        let Some(entry) = self.loaded_entries.get(&id).copied() else {
            return Ok(None);
        };
        self.limits
            .validate_depth(depth, id.number, id.generation)?;
        if !stack.insert(id) {
            return Err(PdfError::CircularReference {
                id: id.number,
                gen: id.generation,
            });
        }
        let result = self.read_source_entry(id, entry, stack, depth);
        stack.remove(&id);
        result
    }

    fn read_source_entry(
        &self,
        id: ObjectId,
        entry: XRefEntry,
        stack: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> PdfResult<Option<PdfObject>> {
        match entry {
            XRefEntry::Free { .. } => Ok(None),
            XRefEntry::InUse { offset, .. } => {
                let offset = usize::try_from(offset).map_err(|_| PdfError::InvalidXRef {
                    offset: 0,
                    message: format!("Object {} offset does not fit", id.number),
                })?;
                if offset >= self.raw_data.len() {
                    return Err(PdfError::InvalidXRef {
                        offset,
                        message: format!("Object {} offset is past the signed bytes", id.number),
                    });
                }
                let mut parser = Parser::at_offset(&self.raw_data, offset);
                let (parsed_id, obj) = parser.parse_indirect_object()?;
                if parsed_id.number != id.number {
                    return Err(PdfError::InvalidXRef {
                        offset,
                        message: format!(
                            "XRef pointed to object {}, but found object {}",
                            id.number, parsed_id
                        ),
                    });
                }
                Ok(Some(obj))
            }
            XRefEntry::Compressed {
                container_id,
                index_in_stream,
            } => {
                let container = self
                    .source_object(ObjectId::new(container_id), stack, depth + 1)?
                    .ok_or(PdfError::ObjectNotFound {
                        id: container_id,
                        gen: 0,
                    })?;
                let PdfObject::Stream(stream) = container else {
                    return Err(PdfError::TypeMismatch {
                        id: container_id,
                        gen: 0,
                        expected: "Stream",
                        found: "Non-stream object",
                    });
                };
                Ok(Some(self.extract_from_obj_stm(&stream, index_in_stream)?))
            }
        }
    }

    /// Rewrites every object. A SHA-256 attestation is sealed against the new bytes.
    fn save_rewritten(&mut self) -> PdfResult<Vec<u8>> {
        let mut out = Vec::new();
        let mut writer = Writer::new(&mut out);

        writer.write_header(&self.write_version)?;

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

        // Active actions are removed before the rewrite so the saved file does not carry them.
        crate::security::active::neutralize_active_content(self)?;

        // Write all objects present in the objects map sorted by number
        let mut sorted_keys: Vec<ObjectId> = self.objects.keys().copied().collect();
        sorted_keys.sort_by_key(|id| id.number);
        let highest = sorted_keys.iter().map(|id| id.number).max().unwrap_or(0);

        for id in sorted_keys {
            if let Some(obj) = self.objects.get(&id) {
                let offset = writer.write_indirect_object(id, obj)?;
                offsets.push((id, offset));
            }
        }

        let mut trailer = self.xref.trailer.clone();
        // A full rewrite emits a classic table. Keys that belonged to the
        // previous cross-reference stream are not trailer keys anymore.
        if trailer.get("Type").and_then(|obj| obj.as_name()) == Some("XRef") {
            trailer.remove("Type");
        }
        for key in [
            "W",
            "Index",
            "Filter",
            "DecodeParms",
            "Length",
            "Prev",
            "XRefStm",
        ] {
            trailer.remove(key);
        }
        trailer.insert("Size", i64::from(highest) + 1);
        writer.write_xref_and_trailer(&offsets, &trailer)?;

        // A full rewrite moves every offset, so an existing SHA-256 attestation
        // has to be recomputed against these exact bytes before they leave.
        crate::security::signatures::seal_saved_bytes(&mut out)?;

        Ok(out)
    }

    /// Optimizes the document in-place using reachability analysis, stream recompression,
    /// stream deduplication, and Object Stream (/ObjStm) packing.
    pub fn optimize(
        &mut self,
        options: &crate::ops::OptimizationOptions,
    ) -> PdfResult<crate::ops::OptimizationStats> {
        crate::ops::optimize_document(self, options)
    }

    /// Serializes the document into an optimized, compressed PDF byte vector.
    pub fn save_optimized_to_vec(
        &mut self,
        options: &crate::ops::OptimizationOptions,
    ) -> PdfResult<(Vec<u8>, crate::ops::OptimizationStats)> {
        self.require_modify()?;
        crate::ops::save_optimized_to_vec(self, options)
    }

    /// Extracts document metadata combining `/Info` and `/Root /Metadata` (XMP).
    pub fn get_metadata(&mut self) -> PdfResult<crate::ops::metadata::DocumentMetadata> {
        crate::ops::metadata::extract_metadata(self)
    }

    /// Updates document metadata and synchronizes `/Info` and `/Root /Metadata` (XMP).
    pub fn set_metadata(&mut self, meta: &crate::ops::metadata::DocumentMetadata) -> PdfResult<()> {
        self.require_modify()?;
        crate::ops::metadata::update_metadata(self, meta)
    }

    /// Extracts the hierarchical Document Outline (`/Outlines` / Table of Contents) tree.
    pub fn get_outlines(&mut self) -> PdfResult<Vec<crate::ops::outlines::OutlineItem>> {
        crate::ops::outlines::extract_outlines(self)
    }

    /// Replaces the Document Outline (`/Outlines` / Table of Contents) tree.
    pub fn set_outlines(
        &mut self,
        items: &[crate::ops::outlines::OutlineItem],
    ) -> PdfResult<usize> {
        self.require_assemble()?;
        crate::ops::outlines::set_outlines(self, items)
    }
}

impl PdfDocument {
    fn permission_denied() -> PdfError {
        PdfError::CryptographyError(PERMISSION_DENIED.to_string())
    }

    /// Remembers `/P` for a user password. The owner, and a user whose enforced
    /// bits are all set, stay unrestricted and are not told apart later.
    pub(crate) fn apply_opened_permissions(&mut self, is_owner: bool, permissions: PdfPermissions) {
        if is_owner || permissions.allows_all_enforced() {
            self.user_permissions = None;
        } else {
            self.user_permissions = Some(permissions);
        }
    }

    /// True when this session may copy text. An unrestricted session may.
    pub fn allows_copy(&self) -> bool {
        self.user_permissions
            .map(PdfPermissions::allows_copy)
            .unwrap_or(true)
    }

    fn require_when(&self, allowed: bool) -> PdfResult<()> {
        if allowed {
            Ok(())
        } else {
            Err(Self::permission_denied())
        }
    }

    /// Bit 3. This process has one raster.
    pub fn require_print(&self) -> PdfResult<()> {
        self.require_when(
            self.user_permissions
                .map(PdfPermissions::allows_print)
                .unwrap_or(true),
        )
    }

    /// Bit 5.
    pub fn require_copy(&self) -> PdfResult<()> {
        self.require_when(self.allows_copy())
    }

    /// Bit 4.
    pub fn require_modify(&self) -> PdfResult<()> {
        self.require_when(
            self.user_permissions
                .map(PdfPermissions::allows_modify)
                .unwrap_or(true),
        )
    }

    /// Bit 6.
    pub fn require_annotate(&self) -> PdfResult<()> {
        self.require_when(
            self.user_permissions
                .map(PdfPermissions::allows_annotate)
                .unwrap_or(true),
        )
    }

    /// Bit 9 or bit 6.
    pub fn require_fill(&self) -> PdfResult<()> {
        self.require_when(
            self.user_permissions
                .map(PdfPermissions::allows_fill)
                .unwrap_or(true),
        )
    }

    /// Bits 4 and 6. Used when a field is created or annotations are burned in.
    pub fn require_form_create(&self) -> PdfResult<()> {
        self.require_when(
            self.user_permissions
                .map(PdfPermissions::allows_content_and_annotations)
                .unwrap_or(true),
        )
    }

    /// Flattening annotations burns them into page content, so both bits are required.
    pub fn require_flatten_annotations(&self) -> PdfResult<()> {
        self.require_form_create()
    }

    /// Bit 11 or bit 4.
    pub fn require_assemble(&self) -> PdfResult<()> {
        self.require_when(
            self.user_permissions
                .map(PdfPermissions::allows_assemble)
                .unwrap_or(true),
        )
    }

    /// A restricted user password cannot encrypt the file again.
    pub fn require_security_change(&self) -> PdfResult<()> {
        if self.user_permissions.is_some() {
            Err(Self::permission_denied())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_objstm_count_is_rejected() {
        let doc = PdfDocument::empty();
        let mut dict = PdfDictionary::new();
        dict.insert("N", -1i64);
        dict.insert("First", 0i64);
        let stream = PdfStream::new(dict, Vec::new());
        let error = doc.extract_from_obj_stm(&stream, 0).unwrap_err();
        assert!(error.to_string().contains("ObjStm /N"));
    }

    #[test]
    fn compressed_objects_that_point_at_each_other_are_rejected() {
        let mut doc = PdfDocument::empty();
        let first = ObjectId::new(10);
        let second = ObjectId::new(11);
        doc.xref.entries.insert(
            first,
            XRefEntry::Compressed {
                container_id: second.number,
                index_in_stream: 0,
            },
        );
        doc.xref.entries.insert(
            second,
            XRefEntry::Compressed {
                container_id: first.number,
                index_in_stream: 0,
            },
        );
        let error = doc.get_object(first).unwrap_err();
        assert!(error.to_string().contains("Circular"));
    }

    #[test]
    fn classic_save_roundtrips_a_gap_in_object_numbers() {
        let mut doc = PdfDocument::empty();
        let gap_id = ObjectId::new(4);
        let mut dict = PdfDictionary::new();
        dict.insert("Type", PdfName::new("Font"));
        doc.set_object(gap_id, PdfObject::Dictionary(dict));

        let bytes = doc.save_to_vec().expect("save");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("xref\n0 3\n"), "{text}");
        assert!(text.contains("\n4 1\n"), "{text}");
        assert!(!text.contains("/Type /XRef"));

        let mut loaded = PdfDocument::load(&bytes).expect("load");
        let saved = loaded.get_object(gap_id).expect("object 4");
        assert_eq!(
            saved
                .as_dict()
                .and_then(|d| d.get("Type"))
                .and_then(|o| o.as_name()),
            Some("Font")
        );
        assert!(loaded.get_object(ObjectId::new(1)).is_ok());
    }
}
