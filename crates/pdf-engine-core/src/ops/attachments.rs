//! Embedded files and PDF Portfolio attachments manager (ISO 32000-1 §7.11.4 & §12.5.6.15).
//!
//! Provides inspection, extraction, insertion, and deletion of embedded file streams
//! stored in the document catalog `/Names /EmbeddedFiles` name tree as well as
//! page-level `/FileAttachment` annotations.

use std::collections::{BTreeMap, HashSet};

use sha2::{Digest, Sha256};

use crate::cos::filters::{decode_stream, encode_flate, Compression};
use crate::cos::{
    ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, PdfString,
};
use crate::crypto::md5;
use crate::error::{PdfError, PdfResult};

/// Maximum recursion depth when traversing `/Names /EmbeddedFiles` `/Kids` trees.
const MAX_NAME_TREE_DEPTH: usize = 32;

/// Maximum filename length in characters for an embedded attachment.
const MAX_ATTACHMENT_NAME_LEN: usize = 255;

/// Metadata describing an embedded file attachment in a PDF document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedAttachmentInfo {
    /// Logical key in the `/EmbeddedFiles` name tree (or annotation identifier).
    pub name: String,
    /// Resolved filename from `/UF` or `/F` on the `/Filespec` dictionary.
    pub filename: String,
    /// MIME type extracted from the `/EmbeddedFile` stream `/Subtype` (e.g., `"application/pdf"`).
    pub mime_type: String,
    /// Uncompressed size of the embedded payload in bytes.
    pub size: usize,
    /// Optional descriptive text from `/Filespec` `/Desc`.
    pub description: Option<String>,
    /// Optional creation timestamp string from `/Params /CreationDate`.
    pub creation_date: Option<String>,
    /// Optional modification timestamp string from `/Params /ModDate`.
    pub modification_date: Option<String>,
    /// Optional lowercase hex MD5 checksum (from `/Params /CheckSum` or computed).
    pub checksum_md5: Option<String>,
    /// Lowercase hex SHA-256 digest of the decoded file payload.
    pub checksum_sha256: String,
    /// Origin of the attachment (`"embedded_files"` or `"page_annotation"`).
    pub source: String,
    /// 1-indexed page number when `source == "page_annotation"`.
    pub page_number: Option<usize>,
}

/// Complete extracted attachment including metadata and uncompressed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedAttachment {
    /// Attachment metadata and integrity digests.
    pub info: EmbeddedAttachmentInfo,
    /// Decoded raw file bytes.
    pub data: Vec<u8>,
}

/// Reserves a fresh indirect object ID by immediately inserting `PdfObject::Null`.
fn reserve_object_id(doc: &mut PdfDocument) -> ObjectId {
    let id = doc.alloc_object_id();
    doc.set_object(id, PdfObject::Null);
    id
}

/// Encodes a Rust UTF-8 string into a `PdfString` (ASCII/Latin-1 literal if all
/// codepoints fit in `0x20..=0x7E`, otherwise UTF-16BE with `\xFE\xFF` BOM).
fn encode_pdf_text_string(text: &str) -> PdfString {
    if text.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        PdfString::literal(text.as_bytes().to_vec())
    } else {
        let mut bytes = Vec::with_capacity(2 + text.len() * 2);
        bytes.push(0xFE);
        bytes.push(0xFF);
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        PdfString::hex(bytes)
    }
}

/// Encodes a filename into an ASCII-safe `/F` fallback string per ISO 32000-1 §7.11.3.
fn encode_ascii_filename(filename: &str) -> PdfString {
    let ascii: Vec<u8> = filename
        .chars()
        .map(|ch| {
            if ch.is_ascii() && (0x20..=0x7E).contains(&(ch as u8)) && ch != '/' && ch != '\\' {
                ch as u8
            } else {
                b'_'
            }
        })
        .collect();
    PdfString::literal(ascii)
}

/// Sanitizes and validates an attachment filename, stripping directory components.
fn sanitize_attachment_name(raw_name: &str) -> PdfResult<String> {
    let trimmed = raw_name.trim();
    if trimmed.is_empty() {
        return Err(PdfError::OperationError(
            "Attachment name cannot be empty".to_string(),
        ));
    }
    let basename = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    if basename.is_empty() || basename == "." || basename == ".." {
        return Err(PdfError::OperationError(format!(
            "Invalid attachment filename '{raw_name}'"
        )));
    }
    if basename.chars().count() > MAX_ATTACHMENT_NAME_LEN {
        return Err(PdfError::OperationError(format!(
            "Attachment filename exceeds maximum length of {MAX_ATTACHMENT_NAME_LEN} characters"
        )));
    }
    if basename.chars().any(|c| c.is_control()) {
        return Err(PdfError::OperationError(
            "Attachment filename cannot contain control characters".to_string(),
        ));
    }
    Ok(basename.to_string())
}

/// Normalizes and validates a MIME type string.
fn normalize_mime_type(mime_type: Option<&str>) -> PdfResult<String> {
    let raw = mime_type.unwrap_or("application/octet-stream").trim();
    if raw.is_empty() {
        return Ok("application/octet-stream".to_string());
    }
    if raw.len() > 128 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(PdfError::OperationError(format!(
            "Invalid MIME type '{raw}'"
        )));
    }
    Ok(raw.to_string())
}

/// Formats bytes as a lowercase hexadecimal string.
fn to_hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Resolves a `PdfObject` (following an indirect reference if needed) into a cloned `PdfDictionary`.
fn resolve_dict(doc: &mut PdfDocument, obj: &PdfObject) -> PdfResult<Option<PdfDictionary>> {
    match obj {
        PdfObject::Dictionary(d) => Ok(Some(d.clone())),
        PdfObject::Reference(id) => {
            let resolved = doc.get_object(*id)?;
            Ok(resolved.as_dict().cloned())
        }
        _ => Ok(None),
    }
}

/// Resolves a `PdfObject` (following an indirect reference if needed) into a cloned `PdfStream`.
fn resolve_stream(doc: &mut PdfDocument, obj: &PdfObject) -> PdfResult<Option<PdfStream>> {
    match obj {
        PdfObject::Stream(s) => Ok(Some(s.clone())),
        PdfObject::Reference(id) => {
            let resolved = doc.get_object(*id)?;
            match resolved {
                PdfObject::Stream(s) => Ok(Some(s)),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// Decodes a `PdfStream`'s content according to its `/Filter` and `/DecodeParms`.
fn decode_pdf_stream(doc: &PdfDocument, stream: &PdfStream) -> PdfResult<Vec<u8>> {
    let filter_name = stream
        .dict
        .get("Filter")
        .and_then(|f| {
            f.as_name().or_else(|| {
                f.as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|o| o.as_name())
            })
        })
        .unwrap_or("");
    let decode_parms = stream.dict.get("DecodeParms").and_then(|p| p.as_dict());
    decode_stream(filter_name, decode_parms, &stream.content, &doc.limits)
}

/// Recursively collects `(key_name, filespec_obj)` entries from a Name Tree node.
fn collect_name_tree_entries(
    doc: &mut PdfDocument,
    node_dict: &PdfDictionary,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
    out: &mut Vec<(String, PdfObject)>,
) -> PdfResult<()> {
    if depth > MAX_NAME_TREE_DEPTH {
        return Ok(());
    }

    if let Some(names_obj) = node_dict.get("Names").cloned() {
        let arr_opt = match names_obj {
            PdfObject::Array(arr) => Some(arr),
            PdfObject::Reference(id) => doc.get_object(id)?.as_array().map(|a| a.to_vec()),
            _ => None,
        };
        if let Some(arr) = arr_opt {
            let mut idx = 0;
            while idx + 1 < arr.len() {
                let key_str = match &arr[idx] {
                    PdfObject::String(s) => s.to_string_lossy(),
                    PdfObject::Reference(id) => doc
                        .get_object(*id)?
                        .as_string()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let val_obj = arr[idx + 1].clone();
                if !key_str.is_empty() {
                    out.push((key_str, val_obj));
                }
                idx += 2;
            }
        }
    }

    if let Some(kids_obj) = node_dict.get("Kids").cloned() {
        let kids_opt = match kids_obj {
            PdfObject::Array(arr) => Some(arr),
            PdfObject::Reference(id) => doc.get_object(id)?.as_array().map(|a| a.to_vec()),
            _ => None,
        };
        if let Some(kids) = kids_opt {
            for kid in kids {
                match kid {
                    PdfObject::Reference(kid_id) => {
                        if !visited.insert(kid_id) {
                            continue;
                        }
                        if let Some(kid_dict) = doc.get_object(kid_id)?.as_dict().cloned() {
                            collect_name_tree_entries(doc, &kid_dict, visited, depth + 1, out)?;
                        }
                    }
                    PdfObject::Dictionary(kid_dict) => {
                        collect_name_tree_entries(doc, &kid_dict, visited, depth + 1, out)?;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Decodes a `/Filespec` dictionary into an [`EmbeddedAttachment`].
fn decode_filespec(
    doc: &mut PdfDocument,
    logical_name: &str,
    filespec_dict: &PdfDictionary,
    source: &str,
    page_number: Option<usize>,
) -> PdfResult<Option<EmbeddedAttachment>> {
    // Resolve filename: prefer `/UF` (Unicode filename), then `/F`, then `/DOS`/`/Unix`/`/Mac`, then `logical_name`.
    let filename = filespec_dict
        .get("UF")
        .or_else(|| filespec_dict.get("F"))
        .or_else(|| filespec_dict.get("Unix"))
        .or_else(|| filespec_dict.get("DOS"))
        .or_else(|| filespec_dict.get("Mac"))
        .and_then(|o| o.as_string())
        .map(|s| s.to_string_lossy())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| logical_name.to_string());

    let description = filespec_dict
        .get("Desc")
        .and_then(|o| o.as_string())
        .map(|s| s.to_string_lossy())
        .filter(|s| !s.is_empty());

    let Some(ef_obj) = filespec_dict.get("EF").cloned() else {
        return Ok(None);
    };
    let Some(ef_dict) = resolve_dict(doc, &ef_obj)? else {
        return Ok(None);
    };

    let stream_ref_or_obj = ef_dict
        .get("UF")
        .or_else(|| ef_dict.get("F"))
        .or_else(|| ef_dict.get("Unix"))
        .or_else(|| ef_dict.get("DOS"))
        .or_else(|| ef_dict.get("Mac"))
        .cloned();

    let Some(stream_target) = stream_ref_or_obj else {
        return Ok(None);
    };
    let Some(stream) = resolve_stream(doc, &stream_target)? else {
        return Ok(None);
    };

    let data = decode_pdf_stream(doc, &stream)?;

    let mime_type = stream
        .dict
        .get("Subtype")
        .and_then(|o| o.as_name())
        .filter(|s| !s.is_empty())
        .unwrap_or("application/octet-stream")
        .to_string();

    let mut creation_date = None;
    let mut modification_date = None;
    let mut checksum_md5 = None;

    if let Some(params_obj) = stream.dict.get("Params").cloned() {
        if let Some(params_dict) = resolve_dict(doc, &params_obj)? {
            creation_date = params_dict
                .get("CreationDate")
                .and_then(|o| o.as_string())
                .map(|s| s.to_string_lossy());
            modification_date = params_dict
                .get("ModDate")
                .and_then(|o| o.as_string())
                .map(|s| s.to_string_lossy());
            if let Some(cs) = params_dict.get("CheckSum").and_then(|o| o.as_string()) {
                if !cs.bytes.is_empty() {
                    checksum_md5 = Some(to_hex_lower(&cs.bytes));
                }
            }
        }
    }

    if checksum_md5.is_none() {
        checksum_md5 = Some(to_hex_lower(&md5(&data)));
    }
    let sha256_bytes = Sha256::digest(&data);
    let checksum_sha256 = to_hex_lower(&sha256_bytes);

    let name = if logical_name.is_empty() {
        filename.clone()
    } else {
        logical_name.to_string()
    };

    Ok(Some(EmbeddedAttachment {
        info: EmbeddedAttachmentInfo {
            name,
            filename,
            mime_type,
            size: data.len(),
            description,
            creation_date,
            modification_date,
            checksum_md5,
            checksum_sha256,
            source: source.to_string(),
            page_number,
        },
        data,
    }))
}

/// Collects all embedded attachments in the document (both Catalog `/Names /EmbeddedFiles`
/// and page-level `/FileAttachment` annotations).
fn collect_all_attachments(doc: &mut PdfDocument) -> PdfResult<Vec<EmbeddedAttachment>> {
    let mut results = Vec::new();
    let mut seen_filespec_ids = HashSet::new();

    // 1. Inspect `/Catalog` -> `/Names` -> `/EmbeddedFiles`
    let catalog_names_obj = doc.catalog().ok().and_then(|cat| cat.get("Names").cloned());

    if let Some(names_obj) = catalog_names_obj {
        if let Some(names_dict) = resolve_dict(doc, &names_obj)? {
            if let Some(ef_tree_obj) = names_dict.get("EmbeddedFiles").cloned() {
                if let Some(ef_tree_dict) = resolve_dict(doc, &ef_tree_obj)? {
                    let mut visited = HashSet::new();
                    let mut entries = Vec::new();
                    collect_name_tree_entries(doc, &ef_tree_dict, &mut visited, 0, &mut entries)?;

                    for (key, filespec_obj) in entries {
                        if let PdfObject::Reference(fs_id) = &filespec_obj {
                            seen_filespec_ids.insert(*fs_id);
                        }
                        if let Some(fs_dict) = resolve_dict(doc, &filespec_obj)? {
                            if let Some(att) =
                                decode_filespec(doc, &key, &fs_dict, "embedded_files", None)?
                            {
                                results.push(att);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Inspect page-level `/FileAttachment` annotations
    let page_ids = doc.get_pages().unwrap_or_default();
    for (page_idx, page_id) in page_ids.into_iter().enumerate() {
        let page_num = page_idx + 1;
        let annots_obj = doc
            .get_object(page_id)
            .ok()
            .and_then(|o| o.as_dict().and_then(|d| d.get("Annots").cloned()));

        let Some(annots_obj) = annots_obj else {
            continue;
        };
        let annots_arr = match annots_obj {
            PdfObject::Array(arr) => Some(arr),
            PdfObject::Reference(id) => doc
                .get_object(id)
                .ok()
                .and_then(|o| o.as_array().map(|a| a.to_vec())),
            _ => None,
        };
        let Some(annots) = annots_arr else {
            continue;
        };

        for annot_item in annots {
            let Some(annot_dict) = resolve_dict(doc, &annot_item)? else {
                continue;
            };
            if annot_dict.get("Subtype").and_then(|o| o.as_name()) != Some("FileAttachment") {
                continue;
            }
            let Some(fs_obj) = annot_dict.get("FS").cloned() else {
                continue;
            };
            if let PdfObject::Reference(fs_id) = &fs_obj {
                if !seen_filespec_ids.insert(*fs_id) {
                    continue;
                }
            }
            let fallback_name = annot_dict
                .get("Contents")
                .or_else(|| annot_dict.get("NM"))
                .and_then(|o| o.as_string())
                .map(|s| s.to_string_lossy())
                .unwrap_or_default();

            if let Some(fs_dict) = resolve_dict(doc, &fs_obj)? {
                if let Some(att) = decode_filespec(
                    doc,
                    &fallback_name,
                    &fs_dict,
                    "page_annotation",
                    Some(page_num),
                )? {
                    results.push(att);
                }
            }
        }
    }

    Ok(results)
}

/// Lists all embedded file attachments in the document without cloning their binary payloads to callers.
pub fn list_attachments(doc: &mut PdfDocument) -> PdfResult<Vec<EmbeddedAttachmentInfo>> {
    let all = collect_all_attachments(doc)?;
    Ok(all.into_iter().map(|att| att.info).collect())
}

/// Extracts a specific embedded file attachment by logical name or filename.
pub fn extract_attachment(doc: &mut PdfDocument, name: &str) -> PdfResult<EmbeddedAttachment> {
    let target = name.trim();
    if target.is_empty() {
        return Err(PdfError::OperationError(
            "Attachment name cannot be empty".to_string(),
        ));
    }
    let all = collect_all_attachments(doc)?;
    for att in all {
        if att.info.name == target || att.info.filename == target {
            return Ok(att);
        }
    }
    Err(PdfError::OperationError(format!(
        "Attachment '{target}' not found in document"
    )))
}

/// Embeds a file into the document's `/Names /EmbeddedFiles` name tree (ISO 32000-1 §7.11.4).
///
/// If an attachment with the same logical name already exists in `/EmbeddedFiles`,
/// it is atomically replaced.
pub fn add_attachment(
    doc: &mut PdfDocument,
    name: &str,
    data: &[u8],
    mime_type: Option<&str>,
    description: Option<&str>,
) -> PdfResult<EmbeddedAttachmentInfo> {
    let clean_name = sanitize_attachment_name(name)?;
    let clean_mime = normalize_mime_type(mime_type)?;

    if data.len() > doc.limits.max_stream_decompressed_bytes {
        return Err(PdfError::SecurityLimitExceeded(format!(
            "Attachment size ({} bytes) exceeds maximum allowed stream size ({} bytes)",
            data.len(),
            doc.limits.max_stream_decompressed_bytes
        )));
    }

    let catalog_id = doc.catalog_id().ok_or_else(|| PdfError::InvalidXRef {
        offset: 0,
        message: "Document trailer missing /Root reference".into(),
    })?;

    let compressed = encode_flate(data, Compression::default())?;
    let md5_digest = md5(data);
    let sha256_digest = Sha256::digest(data);

    // 1. Create the `/EmbeddedFile` stream object
    let ef_stream_id = reserve_object_id(doc);
    let mut params_dict = PdfDictionary::new();
    params_dict.insert("Size", PdfObject::Integer(data.len() as i64));
    params_dict.insert(
        "CheckSum",
        PdfObject::String(PdfString::hex(md5_digest.to_vec())),
    );

    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Type", PdfObject::Name(PdfName::new("EmbeddedFile")));
    stream_dict.insert("Subtype", PdfObject::Name(PdfName::new(clean_mime.clone())));
    stream_dict.insert("Filter", PdfObject::Name(PdfName::new("FlateDecode")));
    stream_dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    stream_dict.insert("Params", PdfObject::Dictionary(params_dict));

    doc.set_object(
        ef_stream_id,
        PdfObject::Stream(PdfStream::new(stream_dict, compressed)),
    );

    // 2. Create the `/Filespec` dictionary object
    let filespec_id = reserve_object_id(doc);
    let mut ef_dict = PdfDictionary::new();
    ef_dict.insert("F", PdfObject::Reference(ef_stream_id));
    ef_dict.insert("UF", PdfObject::Reference(ef_stream_id));

    let mut filespec_dict = PdfDictionary::new();
    filespec_dict.insert("Type", PdfObject::Name(PdfName::new("Filespec")));
    filespec_dict.insert("F", PdfObject::String(encode_ascii_filename(&clean_name)));
    filespec_dict.insert("UF", PdfObject::String(encode_pdf_text_string(&clean_name)));
    filespec_dict.insert("EF", PdfObject::Dictionary(ef_dict));
    filespec_dict.insert(
        "AFRelationship",
        PdfObject::Name(PdfName::new("Supplement")),
    );

    let clean_desc = description
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(ref desc_text) = clean_desc {
        filespec_dict.insert("Desc", PdfObject::String(encode_pdf_text_string(desc_text)));
    }

    doc.set_object(filespec_id, PdfObject::Dictionary(filespec_dict));

    // 3. Collect existing `/Names /EmbeddedFiles` entries and insert/replace in sorted order
    let mut entries_map: BTreeMap<String, PdfObject> = BTreeMap::new();
    let existing_names_obj = doc.catalog().ok().and_then(|cat| cat.get("Names").cloned());

    let mut names_dict = if let Some(ref obj) = existing_names_obj {
        resolve_dict(doc, obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    if let Some(ef_tree_obj) = names_dict.get("EmbeddedFiles").cloned() {
        if let Some(ef_tree_dict) = resolve_dict(doc, &ef_tree_obj)? {
            let mut visited = HashSet::new();
            let mut existing_entries = Vec::new();
            collect_name_tree_entries(doc, &ef_tree_dict, &mut visited, 0, &mut existing_entries)?;
            for (k, v) in existing_entries {
                entries_map.insert(k, v);
            }
        }
    }

    entries_map.insert(clean_name.clone(), PdfObject::Reference(filespec_id));

    let mut flat_names_array = PdfArray::with_capacity(entries_map.len() * 2);
    let mut af_refs = PdfArray::with_capacity(entries_map.len());
    for (k, v) in &entries_map {
        flat_names_array.push(PdfObject::String(encode_pdf_text_string(k)));
        flat_names_array.push(v.clone());
        if let PdfObject::Reference(id) = v {
            af_refs.push(PdfObject::Reference(*id));
        }
    }

    let mut new_ef_tree = PdfDictionary::new();
    new_ef_tree.insert("Names", PdfObject::Array(flat_names_array));

    let ef_tree_id = match names_dict.get("EmbeddedFiles") {
        Some(PdfObject::Reference(id)) => *id,
        _ => reserve_object_id(doc),
    };
    doc.set_object(ef_tree_id, PdfObject::Dictionary(new_ef_tree));
    names_dict.insert("EmbeddedFiles", PdfObject::Reference(ef_tree_id));

    let names_id = match existing_names_obj {
        Some(PdfObject::Reference(id)) => id,
        _ => reserve_object_id(doc),
    };
    doc.set_object(names_id, PdfObject::Dictionary(names_dict));

    let mut cat_dict = doc.catalog()?;
    cat_dict.insert("Names", PdfObject::Reference(names_id));
    cat_dict.insert("AF", PdfObject::Array(af_refs));
    doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));

    Ok(EmbeddedAttachmentInfo {
        name: clean_name.clone(),
        filename: clean_name,
        mime_type: clean_mime,
        size: data.len(),
        description: clean_desc,
        creation_date: None,
        modification_date: None,
        checksum_md5: Some(to_hex_lower(&md5_digest)),
        checksum_sha256: to_hex_lower(&sha256_digest),
        source: "embedded_files".to_string(),
        page_number: None,
    })
}

/// Removes an embedded file attachment by logical name or filename from the document.
///
/// Purges the entry from `/Names /EmbeddedFiles`, `/Catalog /AF`, any matching page
/// `/FileAttachment` annotations, and removes the underlying `/Filespec` and
/// `/EmbeddedFile` stream objects.
pub fn delete_attachment(doc: &mut PdfDocument, name: &str) -> PdfResult<bool> {
    let target = name.trim();
    if target.is_empty() {
        return Err(PdfError::OperationError(
            "Attachment name cannot be empty".to_string(),
        ));
    }

    let mut removed_any = false;
    let mut removed_filespec_ids: Vec<ObjectId> = Vec::new();

    // 1. Update `/Catalog` -> `/Names` -> `/EmbeddedFiles`
    let existing_names_obj = doc.catalog().ok().and_then(|cat| cat.get("Names").cloned());

    if let Some(ref names_obj) = existing_names_obj {
        if let Some(mut names_dict) = resolve_dict(doc, names_obj)? {
            if let Some(ef_tree_obj) = names_dict.get("EmbeddedFiles").cloned() {
                if let Some(ef_tree_dict) = resolve_dict(doc, &ef_tree_obj)? {
                    let mut visited = HashSet::new();
                    let mut existing_entries = Vec::new();
                    collect_name_tree_entries(
                        doc,
                        &ef_tree_dict,
                        &mut visited,
                        0,
                        &mut existing_entries,
                    )?;

                    let mut kept_entries: Vec<(String, PdfObject)> = Vec::new();
                    for (k, v) in existing_entries {
                        let mut matches_target = k == target;
                        if !matches_target {
                            if let Some(fs_dict) = resolve_dict(doc, &v)? {
                                let fname = fs_dict
                                    .get("UF")
                                    .or_else(|| fs_dict.get("F"))
                                    .and_then(|o| o.as_string())
                                    .map(|s| s.to_string_lossy())
                                    .unwrap_or_default();
                                if fname == target {
                                    matches_target = true;
                                }
                            }
                        }

                        if matches_target {
                            removed_any = true;
                            if let PdfObject::Reference(fs_id) = v {
                                removed_filespec_ids.push(fs_id);
                            }
                        } else {
                            kept_entries.push((k, v));
                        }
                    }

                    if removed_any {
                        if kept_entries.is_empty() {
                            names_dict.remove("EmbeddedFiles");
                            if let PdfObject::Reference(ef_id) = ef_tree_obj {
                                doc.objects.remove(&ef_id);
                                doc.xref.entries.remove(&ef_id);
                            }
                        } else {
                            let mut flat_names_array =
                                PdfArray::with_capacity(kept_entries.len() * 2);
                            for (k, v) in &kept_entries {
                                flat_names_array.push(PdfObject::String(encode_pdf_text_string(k)));
                                flat_names_array.push(v.clone());
                            }
                            let mut new_ef_tree = PdfDictionary::new();
                            new_ef_tree.insert("Names", PdfObject::Array(flat_names_array));
                            let ef_tree_id = match ef_tree_obj {
                                PdfObject::Reference(id) => id,
                                _ => reserve_object_id(doc),
                            };
                            doc.set_object(ef_tree_id, PdfObject::Dictionary(new_ef_tree));
                            names_dict.insert("EmbeddedFiles", PdfObject::Reference(ef_tree_id));
                        }

                        let names_id = match names_obj {
                            PdfObject::Reference(id) => *id,
                            _ => reserve_object_id(doc),
                        };
                        doc.set_object(names_id, PdfObject::Dictionary(names_dict));
                        if let (Some(cat_id), Ok(mut cat_dict)) = (doc.catalog_id(), doc.catalog())
                        {
                            cat_dict.insert("Names", PdfObject::Reference(names_id));
                            doc.set_object(cat_id, PdfObject::Dictionary(cat_dict));
                        }
                    }
                }
            }
        }
    }

    // 2. Also check page-level `/FileAttachment` annotations
    let page_ids = doc.get_pages().unwrap_or_default();
    for page_id in page_ids {
        let annots_obj = doc
            .get_object(page_id)
            .ok()
            .and_then(|o| o.as_dict().and_then(|d| d.get("Annots").cloned()));

        let Some(PdfObject::Array(annots)) = annots_obj else {
            continue;
        };

        let mut kept_annots = Vec::with_capacity(annots.len());
        let mut page_modified = false;
        let mut removed_annot_ids = Vec::new();

        for annot_item in annots {
            let mut should_remove_annot = false;
            if let Some(annot_dict) = resolve_dict(doc, &annot_item)? {
                if annot_dict.get("Subtype").and_then(|o| o.as_name()) == Some("FileAttachment") {
                    if let Some(fs_obj) = annot_dict.get("FS").cloned() {
                        if let Some(fs_dict) = resolve_dict(doc, &fs_obj)? {
                            let fname = fs_dict
                                .get("UF")
                                .or_else(|| fs_dict.get("F"))
                                .and_then(|o| o.as_string())
                                .map(|s| s.to_string_lossy())
                                .unwrap_or_default();
                            if fname == target {
                                should_remove_annot = true;
                                if let PdfObject::Reference(fs_id) = fs_obj {
                                    removed_filespec_ids.push(fs_id);
                                }
                            }
                        }
                    }
                }
            }
            if should_remove_annot {
                removed_any = true;
                page_modified = true;
                if let PdfObject::Reference(annot_id) = annot_item {
                    removed_annot_ids.push(annot_id);
                }
            } else {
                kept_annots.push(annot_item);
            }
        }

        if page_modified {
            if let Ok(mut page_obj) = doc.get_object(page_id) {
                if let Some(page_dict) = page_obj.as_dict_mut() {
                    if kept_annots.is_empty() {
                        page_dict.remove("Annots");
                    } else {
                        page_dict.insert("Annots", PdfObject::Array(kept_annots));
                    }
                }
                doc.set_object(page_id, page_obj);
            }
            for annot_id in removed_annot_ids {
                doc.objects.remove(&annot_id);
                doc.xref.entries.remove(&annot_id);
            }
        }
    }

    // 3. Purge removed `/Filespec` objects, their `/EmbeddedFile` streams, and `/Catalog /AF` references
    for fs_id in &removed_filespec_ids {
        if let Some(fs_dict) = doc
            .get_object(*fs_id)
            .ok()
            .and_then(|o| o.as_dict().cloned())
        {
            if let Some(ef_obj) = fs_dict.get("EF").cloned() {
                if let Some(ef_dict) = resolve_dict(doc, &ef_obj)? {
                    for key in ["F", "UF", "DOS", "Mac", "Unix"] {
                        if let Some(PdfObject::Reference(stream_id)) = ef_dict.get(key) {
                            doc.objects.remove(stream_id);
                            doc.xref.entries.remove(stream_id);
                        }
                    }
                }
            }
        }
        doc.objects.remove(fs_id);
        doc.xref.entries.remove(fs_id);
    }

    if !removed_filespec_ids.is_empty() {
        if let (Some(cat_id), Ok(mut cat_dict)) = (doc.catalog_id(), doc.catalog()) {
            if let Some(PdfObject::Array(af_arr)) = cat_dict.get_mut("AF") {
                af_arr.retain(|item| match item {
                    PdfObject::Reference(id) => !removed_filespec_ids.contains(id),
                    _ => true,
                });
                if af_arr.is_empty() {
                    cat_dict.remove("AF");
                }
                doc.set_object(cat_id, PdfObject::Dictionary(cat_dict));
            }
        }
    }

    Ok(removed_any)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_minimal_doc() -> PdfDocument {
        let mut doc = PdfDocument::empty();
        let page_id = reserve_object_id(&mut doc);
        let mut page_dict = PdfDictionary::new();
        page_dict.insert("Type", PdfObject::Name(PdfName::new("Page")));
        page_dict.insert("Parent", PdfObject::Reference(ObjectId::new(2)));
        page_dict.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(612),
                PdfObject::Integer(792),
            ]),
        );
        doc.set_object(page_id, PdfObject::Dictionary(page_dict));

        if let Some(pages_dict) = doc
            .objects
            .get_mut(&ObjectId::new(2))
            .and_then(|o| o.as_dict_mut())
        {
            pages_dict.insert(
                "Kids",
                PdfObject::Array(vec![PdfObject::Reference(page_id)]),
            );
            pages_dict.insert("Count", PdfObject::Integer(1));
        }
        doc
    }

    #[test]
    fn test_add_list_extract_and_delete_attachments_roundtrip() {
        let mut doc = create_minimal_doc();
        assert!(list_attachments(&mut doc).unwrap().is_empty());

        let xml_payload =
            br#"<?xml version="1.0" encoding="UTF-8"?><Invoice><ID>INV-2026-001</ID></Invoice>"#;
        let csv_payload = b"id,concept,amount\n1,Consulting,1500.00\n2,License,499.00\n";

        let info1 = add_attachment(
            &mut doc,
            "factur-x.xml",
            xml_payload,
            Some("text/xml"),
            Some("Factur-X / ZUGFeRD Electronic Invoice XML"),
        )
        .unwrap();
        assert_eq!(info1.name, "factur-x.xml");
        assert_eq!(info1.mime_type, "text/xml");
        assert_eq!(info1.size, xml_payload.len());

        let info2 = add_attachment(
            &mut doc,
            "anexo_financiero_2026.csv",
            csv_payload,
            Some("text/csv"),
            Some("Desglose de costos — Año 2026"),
        )
        .unwrap();
        assert_eq!(info2.name, "anexo_financiero_2026.csv");
        assert_eq!(info2.size, csv_payload.len());

        // Save and reload to verify COS serialization and active-content scrubber compatibility
        let saved_bytes = doc.save_to_vec().unwrap();
        let mut reloaded = PdfDocument::load(&saved_bytes).unwrap();

        let listed = list_attachments(&mut reloaded).unwrap();
        assert_eq!(listed.len(), 2);
        // Lexicographically sorted in the Name Tree: "anexo_financiero_2026.csv" then "factur-x.xml"
        assert_eq!(listed[0].name, "anexo_financiero_2026.csv");
        assert_eq!(listed[0].mime_type, "text/csv");
        assert_eq!(
            listed[0].description.as_deref(),
            Some("Desglose de costos — Año 2026")
        );
        assert_eq!(listed[1].name, "factur-x.xml");
        assert_eq!(listed[1].mime_type, "text/xml");

        // Extract binary payloads and verify byte-for-byte equality
        let extracted_xml = extract_attachment(&mut reloaded, "factur-x.xml").unwrap();
        assert_eq!(extracted_xml.data, xml_payload);
        assert_eq!(extracted_xml.info.checksum_sha256, info1.checksum_sha256);

        let extracted_csv = extract_attachment(&mut reloaded, "anexo_financiero_2026.csv").unwrap();
        assert_eq!(extracted_csv.data, csv_payload);

        // Delete one attachment and verify the other remains intact
        assert!(delete_attachment(&mut reloaded, "factur-x.xml").unwrap());
        assert!(!delete_attachment(&mut reloaded, "factur-x.xml").unwrap());

        let remaining = list_attachments(&mut reloaded).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "anexo_financiero_2026.csv");
    }

    #[test]
    fn test_attachment_path_traversal_and_invalid_inputs_rejected() {
        let mut doc = create_minimal_doc();
        assert!(add_attachment(&mut doc, "   ", b"data", None, None).is_err());
        assert!(add_attachment(&mut doc, "../", b"data", None, None).is_err());

        // Path prefix should be stripped to clean basename
        let info = add_attachment(
            &mut doc,
            "/var/tmp/report.json",
            b"{\"ok\":true}",
            Some("application/json"),
            None,
        )
        .unwrap();
        assert_eq!(info.name, "report.json");
        assert_eq!(info.filename, "report.json");
    }
}
