//! ISO 32000-1 §7.5.7 & §7.5.8 PDF Document Optimization and Stream Compression.
//!
//! Provides production-grade PDF size reduction through:
//! 1. Reachability graph analysis and dead object elimination (Garbage Collection).
//! 2. Lossless content stream recompression using Flate (Zlib Best).
//! 3. Cryptographic hash-based stream deduplication.
//! 4. Object Stream packing (`/ObjStm`, PDF 1.5+) for indirect objects.
//! 5. Compressed cross-reference streams (`/XRef`) replacing verbose ASCII tables.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::cos::filters::{encode_flate, Compression};
use crate::cos::object::{
    ObjectId, PdfDictionary, PdfName, PdfObject, PdfStream,
};
use crate::cos::writer::Writer;
use crate::cos::xref::XRefEntry;
use crate::cos::PdfDocument;
use crate::crypto::sha256::sha256;
use crate::error::{PdfError, PdfResult};

/// Streams larger than this are left unchanged by the zlib-best pass.
const MAX_BEST_RECOMPRESS_BYTES: usize = 1024 * 1024;

/// Smallest object-stream batch. A request of 0 still packs 10, matching the previous floor.
const MIN_OBJECTS_PER_STREAM: usize = 10;

/// Largest object-stream batch. One container cannot hold every object in the document.
const MAX_OBJECTS_PER_STREAM_CAP: usize = 100;

/// Clamps a requested object-stream batch into `10..=100`.
fn clamped_objects_per_stream(requested: usize) -> usize {
    requested.clamp(MIN_OBJECTS_PER_STREAM, MAX_OBJECTS_PER_STREAM_CAP)
}

/// Configuration parameters for PDF document optimization and stream compression.
#[derive(Debug, Clone)]
pub struct OptimizationOptions {
    /// Eliminate unreferenced or orphaned objects from the document graph (Garbage Collection).
    pub remove_unused_objects: bool,
    /// Pack eligible indirect objects into compressed Object Streams (`/ObjStm`, PDF 1.5+).
    pub pack_into_object_streams: bool,
    /// Maximum number of indirect objects to pack into a single `/ObjStm` container. Default is 100.
    pub max_objects_per_stream: usize,
    /// Recompress eligible uncompressed or inefficient content streams with Flate (Zlib Best).
    pub recompress_flate: bool,
    /// Deduplicate identical stream objects and consolidate indirect references.
    pub deduplicate_streams: bool,
    /// Serialize cross-references using a compressed XRef Stream instead of an ASCII table.
    pub use_xref_stream: bool,
}

impl Default for OptimizationOptions {
    fn default() -> Self {
        Self {
            remove_unused_objects: true,
            pack_into_object_streams: true,
            max_objects_per_stream: 100,
            recompress_flate: true,
            deduplicate_streams: true,
            use_xref_stream: true,
        }
    }
}

/// Quantitative results and compression metrics achieved after running document optimization.
#[derive(Debug, Clone, Default)]
pub struct OptimizationStats {
    /// Size of the original document in bytes before optimization.
    pub original_size: usize,
    /// Size of the resulting document in bytes after optimization.
    pub optimized_size: usize,
    /// Absolute bytes saved (`original_size - optimized_size`).
    pub bytes_saved: usize,
    /// Percentage reduction ratio (e.g. 35.5% reduction).
    pub compression_ratio_pct: f64,
    /// Number of orphaned or dead objects removed by garbage collection.
    pub objects_removed: usize,
    /// Number of content streams newly compressed or recompressed.
    pub streams_recompressed: usize,
    /// Number of `/ObjStm` container streams generated.
    pub object_streams_created: usize,
    /// Number of duplicated stream objects consolidated.
    pub streams_deduplicated: usize,
}

/// Recursively traverses a PDF object to collect all outgoing indirect references.
fn collect_references(obj: &PdfObject, refs: &mut Vec<ObjectId>) {
    match obj {
        PdfObject::Reference(id) => refs.push(*id),
        PdfObject::Array(arr) => {
            for item in arr {
                collect_references(item, refs);
            }
        }
        PdfObject::Dictionary(dict) => {
            for val in dict.0.values() {
                collect_references(val, refs);
            }
        }
        PdfObject::Stream(stream) => {
            for val in stream.dict.0.values() {
                collect_references(val, refs);
            }
        }
        _ => {}
    }
}

/// Recursively updates all indirect references according to a remapping table.
fn replace_references(obj: &mut PdfObject, map: &HashMap<ObjectId, ObjectId>) {
    match obj {
        PdfObject::Reference(id) => {
            if let Some(&new_id) = map.get(id) {
                *id = new_id;
            }
        }
        PdfObject::Array(arr) => {
            for item in arr.iter_mut() {
                replace_references(item, map);
            }
        }
        PdfObject::Dictionary(dict) => {
            for val in dict.0.values_mut() {
                replace_references(val, map);
            }
        }
        PdfObject::Stream(stream) => {
            for val in stream.dict.0.values_mut() {
                replace_references(val, map);
            }
        }
        _ => {}
    }
}

/// Performs reachability analysis from document root nodes and removes unreferenced objects.
pub fn collect_garbage(doc: &mut PdfDocument) -> PdfResult<usize> {
    let mut worklist = Vec::new();

    if let Some(root_ref) = doc.xref.trailer.get("Root").and_then(|r| r.as_reference()) {
        worklist.push(root_ref);
    }
    if let Some(info_ref) = doc.xref.trailer.get("Info").and_then(|r| r.as_reference()) {
        worklist.push(info_ref);
    }
    if let Some(enc_ref) = doc.xref.trailer.get("Encrypt").and_then(|r| r.as_reference()) {
        worklist.push(enc_ref);
    }

    let mut reachable = HashSet::new();

    while let Some(id) = worklist.pop() {
        if !reachable.insert(id) {
            continue;
        }

        if let Ok(obj) = doc.get_object(id) {
            let mut child_refs = Vec::new();
            collect_references(&obj, &mut child_refs);
            for child_id in child_refs {
                if !reachable.contains(&child_id) {
                    worklist.push(child_id);
                }
            }
        }
    }

    let mut all_known_ids: HashSet<ObjectId> = HashSet::new();
    all_known_ids.extend(doc.objects.keys().copied());
    all_known_ids.extend(doc.xref.entries.keys().copied());

    let mut removed_count = 0;
    for id in all_known_ids {
        if !reachable.contains(&id) {
            let was_in_objects = doc.objects.remove(&id).is_some();
            let was_in_xref = doc.xref.entries.remove(&id).is_some();
            if was_in_objects || was_in_xref {
                removed_count += 1;
            }
        }
    }

    Ok(removed_count)
}

/// Identifies identical streams by cryptographic hash and merges redundant references.
pub fn deduplicate_streams(doc: &mut PdfDocument) -> PdfResult<usize> {
    // Map from SHA-256 hash to canonical ObjectId
    let mut content_hashes: HashMap<[u8; 32], ObjectId> = HashMap::new();
    let mut id_remap: HashMap<ObjectId, ObjectId> = HashMap::new();

    let stream_ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(&id, obj)| match obj {
            PdfObject::Stream(_) => Some(id),
            _ => None,
        })
        .collect();

    for id in stream_ids {
        if let Some(PdfObject::Stream(s)) = doc.objects.get(&id) {
            let hash = sha256(&s.content);
            if let Some(&canonical_id) = content_hashes.get(&hash) {
                if canonical_id != id {
                    id_remap.insert(id, canonical_id);
                }
            } else {
                content_hashes.insert(hash, id);
            }
        }
    }

    if id_remap.is_empty() {
        return Ok(0);
    }

    let count = id_remap.len();

    // Remap references in trailer
    let mut trailer_obj = PdfObject::Dictionary(doc.xref.trailer.clone());
    replace_references(&mut trailer_obj, &id_remap);
    if let PdfObject::Dictionary(d) = trailer_obj {
        doc.xref.trailer = d;
    }

    // Remap references in all cached objects
    for obj in doc.objects.values_mut() {
        replace_references(obj, &id_remap);
    }

    // Remove duplicates from document
    for dup_id in id_remap.keys() {
        doc.objects.remove(dup_id);
        doc.xref.entries.remove(dup_id);
    }

    Ok(count)
}

/// Compresses uncompressed or suboptimally compressed streams using Flate (Zlib Best).
pub fn recompress_streams(doc: &mut PdfDocument) -> PdfResult<(usize, usize)> {
    let mut recompressed_count = 0;
    let mut bytes_saved = 0;

    let stream_ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(&id, obj)| match obj {
            PdfObject::Stream(_) => Some(id),
            _ => None,
        })
        .collect();

    for id in stream_ids {
        if let Some(PdfObject::Stream(s)) = doc.objects.get_mut(&id) {
            let filter = s.dict.get("Filter").and_then(|f| f.as_name()).unwrap_or("");

            // Never recompress lossy image formats to preserve lossless quality
            if matches!(
                filter,
                "DCTDecode" | "JPXDecode" | "JBIG2Decode" | "CCITTFaxDecode"
            ) {
                continue;
            }

            let original_len = s.content.len();
            // zlib-best on a large buffer dominates CPU. Leave those streams as they are.
            if original_len > MAX_BEST_RECOMPRESS_BYTES {
                continue;
            }

            if filter.is_empty() || filter == "Identity" {
                // Completely uncompressed stream
                if let Ok(compressed) = encode_flate(&s.content, Compression::best()) {
                    if compressed.len() < original_len {
                        bytes_saved += original_len - compressed.len();
                        s.content = compressed;
                        s.dict.insert("Filter", PdfName::new("FlateDecode"));
                        s.dict.insert("Length", s.content.len() as i64);
                        recompressed_count += 1;
                    }
                }
            } else if filter == "FlateDecode" {
                // Attempt recompression with optimal zlib parameters
                if let Ok(decompressed) =
                    crate::cos::filters::decode_flate(&s.content, None, &doc.limits)
                {
                    if decompressed.len() > MAX_BEST_RECOMPRESS_BYTES {
                        continue;
                    }
                    if let Ok(recompressed) = encode_flate(&decompressed, Compression::best()) {
                        if recompressed.len() < original_len {
                            bytes_saved += original_len - recompressed.len();
                            s.content = recompressed;
                            s.dict.insert("Length", s.content.len() as i64);
                            recompressed_count += 1;
                        }
                    }
                }
            }
        }
    }

    Ok((recompressed_count, bytes_saved))
}

/// Optimizes a `PdfDocument` and serializes the result into a compressed PDF byte vector.
pub fn save_optimized_to_vec(
    doc: &mut PdfDocument,
    options: &OptimizationOptions,
) -> PdfResult<(Vec<u8>, OptimizationStats)> {
    let original_size = if !doc.raw_data().is_empty() {
        doc.raw_data().len()
    } else {
        doc.save_to_vec()?.len()
    };

    let mut working_doc = doc.clone();

    // Ensure all in-use objects are preloaded into memory before optimization
    let all_in_use_ids: Vec<ObjectId> = working_doc
        .xref
        .entries
        .iter()
        .filter_map(|(&id, entry)| match entry {
            XRefEntry::InUse { .. } | XRefEntry::Compressed { .. } => Some(id),
            XRefEntry::Free { .. } => None,
        })
        .collect();

    for id in all_in_use_ids {
        let _ = working_doc.get_object(id);
    }

    // The clone has cached every in-use object. Drop its copy of the source
    // file before the rewrite so the peak is not the original buffer twice.
    // The caller's document is left untouched.
    working_doc.release_retained_file();

    crate::security::active::neutralize_active_content(&mut working_doc)?;

    let mut stats = OptimizationStats {
        original_size,
        ..Default::default()
    };

    // 1. Garbage Collection (Dead Object Elimination)
    if options.remove_unused_objects {
        stats.objects_removed = collect_garbage(&mut working_doc)?;
    }

    // 2. Resource & Stream Deduplication
    if options.deduplicate_streams {
        stats.streams_deduplicated = deduplicate_streams(&mut working_doc)?;
    }

    // 3. Lossless Stream Recompression
    if options.recompress_flate {
        let (recompressed, _) = recompress_streams(&mut working_doc)?;
        stats.streams_recompressed = recompressed;
    }

    // Map of packed object ID -> (container_id, index_in_stream)
    let mut compressed_entries: BTreeMap<ObjectId, (u32, u16)> = BTreeMap::new();

    // 4. Object Stream Packing (/ObjStm)
    if options.pack_into_object_streams {
        let root_id = working_doc.catalog_id();
        let enc_id = working_doc
            .xref
            .trailer
            .get("Encrypt")
            .and_then(|e| e.as_reference());

        // Select candidate objects for Object Stream packing
        let candidate_ids: Vec<ObjectId> = working_doc
            .objects
            .iter()
            .filter_map(|(&id, obj)| {
                if matches!(obj, PdfObject::Stream(_)) {
                    return None;
                }
                if id.generation != 0 {
                    return None;
                }
                if Some(id) == enc_id {
                    return None;
                }
                // Leave root catalog uncompressed to ensure maximum viewer compatibility
                if Some(id) == root_id {
                    return None;
                }
                Some(id)
            })
            .collect();

        let batch_size = clamped_objects_per_stream(options.max_objects_per_stream);
        let chunks: Vec<Vec<ObjectId>> = candidate_ids
            .chunks(batch_size)
            .map(|c| c.to_vec())
            .collect();

        for chunk in chunks {
            let container_id = working_doc.alloc_object_id();
            let mut objects_bytes = Vec::new();
            let mut header_pairs = Vec::with_capacity(chunk.len());

            for (idx, &id) in chunk.iter().enumerate() {
                if let Some(obj) = working_doc.objects.get(&id) {
                    let rel_offset = objects_bytes.len();
                    header_pairs.push((id.number, rel_offset));
                    let mut obj_writer = Writer::new(&mut objects_bytes);
                    obj_writer
                        .write_object(obj)
                        .map_err(|e| PdfError::OperationError(e.to_string()))?;
                    objects_bytes.push(b'\n');

                    compressed_entries
                        .insert(id, (container_id.number, idx as u16));
                }
            }

            let mut header_bytes = Vec::new();
            for (obj_num, rel_offset) in header_pairs {
                header_bytes.extend_from_slice(format!("{} {} ", obj_num, rel_offset).as_bytes());
            }

            let first_offset = header_bytes.len();
            let mut raw_stream_content = header_bytes;
            raw_stream_content.extend_from_slice(&objects_bytes);

            let compressed_content = encode_flate(&raw_stream_content, Compression::best())
                .map_err(|e| PdfError::DecompressionError {
                    filter: "FlateDecode".to_string(),
                    message: e.to_string(),
                })?;

            let mut stream_dict = PdfDictionary::new();
            stream_dict.insert("Type", PdfName::new("ObjStm"));
            stream_dict.insert("N", chunk.len() as i64);
            stream_dict.insert("First", first_offset as i64);
            stream_dict.insert("Filter", PdfName::new("FlateDecode"));
            stream_dict.insert("Length", compressed_content.len() as i64);

            let obj_stream = PdfStream {
                dict: stream_dict,
                content: compressed_content,
            };

            // Remove packed objects from standalone objects map
            for &id in &chunk {
                working_doc.objects.remove(&id);
            }

            // Insert new Object Stream
            working_doc
                .objects
                .insert(container_id, PdfObject::Stream(obj_stream));
            working_doc.xref.entries.insert(
                container_id,
                XRefEntry::InUse {
                    offset: 0,
                    generation: 0,
                },
            );

            stats.object_streams_created += 1;
        }
    }

    // 5. Serialization
    let mut out = Vec::new();
    let mut writer = Writer::new(&mut out);
    writer.write_header("1.7")?;

    let mut final_entries: BTreeMap<ObjectId, XRefEntry> = BTreeMap::new();

    // Write all remaining uncompressed objects (Streams, /ObjStm streams, Root)
    let mut sorted_keys: Vec<ObjectId> = working_doc.objects.keys().copied().collect();
    sorted_keys.sort_by_key(|id| id.number);

    for id in sorted_keys {
        if let Some(obj) = working_doc.objects.get(&id) {
            let offset = writer.write_indirect_object(id, obj)?;
            final_entries.insert(
                id,
                XRefEntry::InUse {
                    offset: offset as u64,
                    generation: id.generation,
                },
            );
        }
    }

    // Record all compressed entries from Object Streams
    for (id, (container_id, index_in_stream)) in compressed_entries {
        final_entries.insert(
            id,
            XRefEntry::Compressed {
                container_id,
                index_in_stream,
            },
        );
    }

    if options.use_xref_stream {
        // PDF 1.5+ compressed XRef Stream
        writer.write_xref_stream(&final_entries, &working_doc.xref.trailer)?;
    } else {
        // Classic ASCII xref table fallback
        let offsets: Vec<(ObjectId, usize)> = final_entries
            .iter()
            .filter_map(|(&id, entry)| match entry {
                XRefEntry::InUse { offset, .. } => Some((id, *offset as usize)),
                _ => None,
            })
            .collect();

        let mut trailer = working_doc.xref.trailer.clone();
        trailer.insert("Size", (final_entries.len() + 1) as i64);
        writer.write_xref_and_trailer(&offsets, &trailer)?;
    }

    stats.optimized_size = out.len();
    stats.bytes_saved = original_size.saturating_sub(stats.optimized_size);
    stats.compression_ratio_pct = if original_size > 0 {
        (stats.bytes_saved as f64 / original_size as f64) * 100.0
    } else {
        0.0
    };

    Ok((out, stats))
}

/// Optimizes a `PdfDocument` in-place, validating the optimized PDF via full round-trip reload.
pub fn optimize_document(
    doc: &mut PdfDocument,
    options: &OptimizationOptions,
) -> PdfResult<OptimizationStats> {
    let (optimized_bytes, stats) = save_optimized_to_vec(doc, options)?;
    *doc = PdfDocument::load_with_limits(&optimized_bytes, doc.limits.clone())?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::PdfArray;

    #[test]
    fn test_optimize_document_roundtrip() {
        let mut doc = PdfDocument::empty();

        // Add 5 uncompressed text streams and orphaned objects
        let dead_obj_1 = doc.alloc_object_id();
        doc.set_object(dead_obj_1, PdfObject::Integer(99999));
        let dead_obj_2 = doc.alloc_object_id();
        doc.set_object(dead_obj_2, PdfObject::String(crate::cos::PdfString::literal("Orphaned data")));

        // Add an active page with an uncompressed content stream
        let pages_id = doc.pages_id().unwrap();
        let page_id = doc.alloc_object_id();
        doc.set_object(page_id, PdfObject::Null);
        let content_id = doc.alloc_object_id();

        let uncompressed_text = b"BT /F1 12 Tf 100 700 Td (Hello World Optimization) Tj ET\n".repeat(50);
        let mut content_dict = PdfDictionary::new();
        content_dict.insert("Length", uncompressed_text.len() as i64);
        doc.set_object(content_id, PdfObject::Stream(PdfStream {
            dict: content_dict,
            content: uncompressed_text.clone(),
        }));

        let mut page_dict = PdfDictionary::new();
        page_dict.insert("Type", PdfName::new("Page"));
        page_dict.insert("Parent", pages_id);
        page_dict.insert("Contents", content_id);
        doc.set_object(page_id, PdfObject::Dictionary(page_dict));

        // Attach page to Pages root
        let mut pages_dict = match doc.get_object(pages_id).unwrap() {
            PdfObject::Dictionary(d) => d,
            _ => panic!("Expected dictionary"),
        };
        let mut kids = PdfArray::new();
        kids.push(PdfObject::Reference(page_id));
        pages_dict.insert("Kids", PdfObject::Array(kids));
        pages_dict.insert("Count", 1i64);
        doc.set_object(pages_id, PdfObject::Dictionary(pages_dict));

        let initial_bytes = doc.save_to_vec().unwrap();
        assert!(initial_bytes.len() > 1000);

        let options = OptimizationOptions {
            remove_unused_objects: true,
            pack_into_object_streams: true,
            max_objects_per_stream: 10,
            recompress_flate: true,
            deduplicate_streams: true,
            use_xref_stream: true,
        };

        let (optimized_bytes, stats) = save_optimized_to_vec(&mut doc, &options).unwrap();

        // Assert size reduction
        assert!(stats.optimized_size < stats.original_size);
        assert!(stats.bytes_saved > 0);
        assert!(stats.compression_ratio_pct > 0.0);
        assert_eq!(stats.objects_removed, 2); // dead_obj_1 and dead_obj_2 removed!
        assert!(stats.streams_recompressed >= 1);

        // Verify roundtrip loading
        let mut loaded_doc = PdfDocument::load(&optimized_bytes).unwrap();
        let pages = loaded_doc.get_pages().unwrap();
        assert_eq!(pages.len(), 1);

        // Verify content extracted matches original
        let extracted_content = loaded_doc.get_page_content_bytes(pages[0]).unwrap();
        assert_eq!(extracted_content, uncompressed_text);

        // Verify dead objects cannot be found
        assert!(loaded_doc.get_object(dead_obj_1).is_err());
        assert!(loaded_doc.get_object(dead_obj_2).is_err());
    }

    #[test]
    fn release_retained_file_drops_source_bytes_after_objects_are_cached() {
        let mut draft = PdfDocument::empty();
        let bytes = draft.save_to_vec().unwrap();
        let mut loaded = PdfDocument::load(&bytes).unwrap();
        let retained = loaded.raw_data().len();
        assert!(retained > 0);

        let ids: Vec<_> = loaded
            .xref
            .entries
            .iter()
            .filter_map(|(&id, entry)| match entry {
                XRefEntry::InUse { .. } | XRefEntry::Compressed { .. } => Some(id),
                XRefEntry::Free { .. } => None,
            })
            .collect();
        for id in ids {
            loaded.get_object(id).unwrap();
        }
        loaded.release_retained_file();
        assert!(loaded.raw_data().is_empty());
        let catalog_id = loaded.catalog_id().unwrap();
        assert!(loaded.get_object(catalog_id).is_ok());

        let mut caller = PdfDocument::load(&bytes).unwrap();
        let options = OptimizationOptions {
            remove_unused_objects: true,
            pack_into_object_streams: false,
            max_objects_per_stream: 10,
            recompress_flate: false,
            deduplicate_streams: false,
            use_xref_stream: false,
        };
        let _ = save_optimized_to_vec(&mut caller, &options).unwrap();
        assert_eq!(caller.raw_data().len(), retained);
    }

    #[test]
    fn test_stream_deduplication() {
        let mut doc = PdfDocument::empty();
        let pages_id = doc.pages_id().unwrap();

        let stream_bytes = b"q /F1 12 Tf 50 700 Td (Shared Header Branding) Tj Q\n";

        // Create stream 1
        let s1_id = doc.alloc_object_id();
        doc.set_object(s1_id, PdfObject::Stream(PdfStream {
            dict: {
                let mut d = PdfDictionary::new();
                d.insert("Length", stream_bytes.len() as i64);
                d
            },
            content: stream_bytes.to_vec(),
        }));

        // Create identical stream 2
        let s2_id = doc.alloc_object_id();
        doc.set_object(s2_id, PdfObject::Stream(PdfStream {
            dict: {
                let mut d = PdfDictionary::new();
                d.insert("Length", stream_bytes.len() as i64);
                d
            },
            content: stream_bytes.to_vec(),
        }));

        // Page 1 points to s1
        let p1_id = doc.alloc_object_id();
        let mut p1_dict = PdfDictionary::new();
        p1_dict.insert("Type", PdfName::new("Page"));
        p1_dict.insert("Parent", pages_id);
        p1_dict.insert("Contents", s1_id);
        doc.set_object(p1_id, PdfObject::Dictionary(p1_dict));

        // Page 2 points to s2
        let p2_id = doc.alloc_object_id();
        let mut p2_dict = PdfDictionary::new();
        p2_dict.insert("Type", PdfName::new("Page"));
        p2_dict.insert("Parent", pages_id);
        p2_dict.insert("Contents", s2_id);
        doc.set_object(p2_id, PdfObject::Dictionary(p2_dict));

        // Update pages tree
        let mut pages_dict = match doc.get_object(pages_id).unwrap() {
            PdfObject::Dictionary(d) => d,
            _ => panic!("Expected dictionary"),
        };
        let mut kids = PdfArray::new();
        kids.push(PdfObject::Reference(p1_id));
        kids.push(PdfObject::Reference(p2_id));
        pages_dict.insert("Kids", PdfObject::Array(kids));
        pages_dict.insert("Count", 2i64);
        doc.set_object(pages_id, PdfObject::Dictionary(pages_dict));

        let options = OptimizationOptions {
            remove_unused_objects: true,
            pack_into_object_streams: true,
            max_objects_per_stream: 50,
            recompress_flate: true,
            deduplicate_streams: true,
            use_xref_stream: true,
        };

        let stats = doc.optimize(&options).unwrap();
        assert_eq!(stats.streams_deduplicated, 1);

        // Verify that both pages now share the exact same content stream ID
        let p1_obj = doc.get_object(p1_id).unwrap();
        let p2_obj = doc.get_object(p2_id).unwrap();

        let p1_contents = match p1_obj {
            PdfObject::Dictionary(d) => d.get("Contents").and_then(|c| c.as_reference()).unwrap(),
            _ => panic!("Expected dict"),
        };
        let p2_contents = match p2_obj {
            PdfObject::Dictionary(d) => d.get("Contents").and_then(|c| c.as_reference()).unwrap(),
            _ => panic!("Expected dict"),
        };

        assert_eq!(p1_contents, p2_contents);
    }

    #[test]
    fn object_stream_batch_is_clamped() {
        assert_eq!(clamped_objects_per_stream(0), 10);
        assert_eq!(clamped_objects_per_stream(50), 50);
        assert_eq!(clamped_objects_per_stream(usize::MAX), 100);
    }

    #[test]
    fn recompress_skips_streams_above_the_best_effort_cap() {
        let mut doc = PdfDocument::empty();
        let id = doc.alloc_object_id();
        let content = vec![b'A'; MAX_BEST_RECOMPRESS_BYTES + 1];
        let mut dict = PdfDictionary::new();
        dict.insert("Length", content.len() as i64);
        doc.set_object(
            id,
            PdfObject::Stream(PdfStream {
                dict,
                content,
            }),
        );

        let (count, saved) = recompress_streams(&mut doc).unwrap();
        assert_eq!(count, 0);
        assert_eq!(saved, 0);
        match doc.get_object(id).unwrap() {
            PdfObject::Stream(stream) => {
                assert!(stream.dict.get("Filter").is_none());
                assert_eq!(stream.content.len(), MAX_BEST_RECOMPRESS_BYTES + 1);
            }
            _ => panic!("expected the oversized stream to stay a stream"),
        }
    }
}
