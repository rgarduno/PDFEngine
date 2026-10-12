//! Document Outline / Interactive Table of Contents (Bookmarks) engine (ISO 32000-1 §12.3.3).
//!
//! Provides lossless extraction, hierarchical traversal, and atomic reconstruction
//! of the `/Outlines` doubly-linked tree in the PDF `/Catalog`.

use std::collections::HashSet;

use crate::cos::{ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfString};
use crate::error::{PdfError, PdfResult};

/// A hierarchical node in the PDF Document Outline (Table of Contents / Bookmarks).
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineItem {
    /// Display title of the bookmark (`/Title`).
    pub title: String,
    /// 1-based target page number when this bookmark jumps to a page in the document.
    pub page_number: Option<usize>,
    /// Optional vertical Y coordinate (`top`) on the target page (`/XYZ` or `/FitH`).
    pub top: Option<f64>,
    /// Optional horizontal X coordinate (`left`) on the target page (`/XYZ`).
    pub left: Option<f64>,
    /// Optional zoom factor on the target page (`/XYZ`, where `0.0` or `None` retains current zoom).
    pub zoom: Option<f64>,
    /// Optional external web link (`http://` or `https://`) when the bookmark triggers a `/URI` action.
    pub uri: Option<String>,
    /// Whether child items are expanded by default (`/Count >= 0`).
    pub is_open: bool,
    /// Bold text style flag (`/F` bit 2).
    pub bold: bool,
    /// Italic text style flag (`/F` bit 1).
    pub italic: bool,
    /// Optional RGB color in `[0.0, 1.0]` (`/C`).
    pub color: Option<[f64; 3]>,
    /// Nested child outline items (`/First` .. `/Last`).
    pub children: Vec<OutlineItem>,
}

impl OutlineItem {
    /// Creates a new bookmark pointing to a 1-based page number.
    pub fn new_page(title: impl Into<String>, page_number: usize) -> Self {
        Self {
            title: title.into(),
            page_number: Some(page_number),
            top: None,
            left: None,
            zoom: None,
            uri: None,
            is_open: true,
            bold: false,
            italic: false,
            color: None,
            children: Vec::new(),
        }
    }

    /// Creates a new bookmark pointing to an external web URI.
    pub fn new_uri(title: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            page_number: None,
            top: None,
            left: None,
            zoom: None,
            uri: Some(uri.into()),
            is_open: true,
            bold: false,
            italic: false,
            color: None,
            children: Vec::new(),
        }
    }

    /// Total number of nodes in this subtree (including `self`).
    pub fn total_nodes(&self) -> usize {
        1 + self.children.iter().map(Self::total_nodes).sum::<usize>()
    }
}

/// Allocates and immediately reserves a new `ObjectId` in `doc`.
fn reserve_object_id(doc: &mut PdfDocument) -> ObjectId {
    let id = doc.alloc_object_id();
    doc.set_object(id, PdfObject::Null);
    id
}

/// Encodes a UTF-8 string into a `PdfString`, using UTF-16BE with BOM (`0xFE 0xFF`) for non-ASCII text.
fn str_to_pdf_string(s: &str) -> PdfString {
    if s.is_ascii() {
        PdfString::literal(s.as_bytes().to_vec())
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        PdfString::literal(bytes)
    }
}

/// Validates that a URI uses an allowed `http://` or `https://` scheme.
fn validate_outline_uri(uri: &str) -> PdfResult<String> {
    let trimmed = uri.trim();
    if trimmed.is_empty() || trimmed.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(PdfError::OperationError(
            "Outline URI must be a non-empty http or https URL.".into(),
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(PdfError::OperationError(
            "Only http and https links are accepted in outline items.".into(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Extracts the hierarchical Document Outline (`/Outlines`) tree from a `PdfDocument`.
pub fn extract_outlines(doc: &mut PdfDocument) -> PdfResult<Vec<OutlineItem>> {
    let catalog = doc.catalog()?;
    let Some(outlines_ref) = catalog.get("Outlines").and_then(|o| o.as_reference()) else {
        return Ok(Vec::new());
    };

    let page_ids = doc.get_pages()?;
    let outlines_obj = doc.get_object(outlines_ref)?;
    let Some(outlines_dict) = outlines_obj.as_dict() else {
        return Ok(Vec::new());
    };

    let Some(first_ref) = outlines_dict.get("First").and_then(|o| o.as_reference()) else {
        return Ok(Vec::new());
    };

    let mut visited = HashSet::new();
    visited.insert(outlines_ref);
    read_outline_siblings(doc, first_ref, &page_ids, &mut visited, 0)
}

fn read_outline_siblings(
    doc: &mut PdfDocument,
    start_id: ObjectId,
    page_ids: &[ObjectId],
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) -> PdfResult<Vec<OutlineItem>> {
    if depth > doc.limits.max_recursion_depth {
        return Err(PdfError::RecursionLimitExceeded {
            id: start_id.number,
            gen: start_id.generation,
            max_depth: doc.limits.max_recursion_depth,
        });
    }

    let mut items = Vec::new();
    let mut current_opt = Some(start_id);

    while let Some(curr_id) = current_opt {
        if !visited.insert(curr_id) {
            break;
        }

        let obj = match doc.get_object(curr_id) {
            Ok(o) => o,
            Err(_) => break,
        };
        let Some(dict) = obj.as_dict().cloned() else {
            break;
        };

        let title = dict
            .get("Title")
            .and_then(|o| o.as_string())
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();

        let count_val = dict.get("Count").and_then(|o| o.as_integer()).unwrap_or(0);
        let is_open = count_val >= 0;

        let flags = dict.get("F").and_then(|o| o.as_integer()).unwrap_or(0);
        let italic = (flags & 1) != 0;
        let bold = (flags & 2) != 0;

        let color = dict.get("C").and_then(|o| o.as_array()).and_then(|arr| {
            if arr.len() == 3 {
                Some([
                    arr[0].as_f64()?.clamp(0.0, 1.0),
                    arr[1].as_f64()?.clamp(0.0, 1.0),
                    arr[2].as_f64()?.clamp(0.0, 1.0),
                ])
            } else {
                None
            }
        });

        let (page_number, left, top, zoom, uri) = parse_outline_target(doc, &dict, page_ids);

        let children = if let Some(first_child) = dict.get("First").and_then(|o| o.as_reference()) {
            read_outline_siblings(doc, first_child, page_ids, visited, depth + 1)?
        } else {
            Vec::new()
        };

        items.push(OutlineItem {
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
            children,
        });

        current_opt = dict.get("Next").and_then(|o| o.as_reference());
    }

    Ok(items)
}

fn parse_outline_target(
    doc: &mut PdfDocument,
    dict: &PdfDictionary,
    page_ids: &[ObjectId],
) -> (
    Option<usize>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<String>,
) {
    // 1. Check direct /Dest entry
    if let Some(dest_obj) = dict.get("Dest").cloned() {
        let resolved = resolve_if_ref(doc, dest_obj);
        if let Some((page, l, t, z)) = parse_dest_array(&resolved, page_ids) {
            return (Some(page), l, t, z, None);
        }
    }

    // 2. Check /A action dictionary
    if let Some(action_obj) = dict.get("A").cloned() {
        let resolved = resolve_if_ref(doc, action_obj);
        if let Some(action_dict) = resolved.as_dict() {
            let subtype = action_dict.get("S").and_then(|s| s.as_name()).unwrap_or("");
            match subtype {
                "GoTo" => {
                    if let Some(d_obj) = action_dict.get("D").cloned() {
                        let d_resolved = resolve_if_ref(doc, d_obj);
                        if let Some((page, l, t, z)) = parse_dest_array(&d_resolved, page_ids) {
                            return (Some(page), l, t, z, None);
                        }
                    }
                }
                "URI" => {
                    if let Some(uri_str) = action_dict
                        .get("URI")
                        .and_then(|u| u.as_string())
                        .map(|s| s.to_string_lossy())
                    {
                        if let Ok(valid_uri) = validate_outline_uri(&uri_str) {
                            return (None, None, None, None, Some(valid_uri));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    (None, None, None, None, None)
}

fn resolve_if_ref(doc: &mut PdfDocument, obj: PdfObject) -> PdfObject {
    match obj {
        PdfObject::Reference(id) => doc.get_object(id).unwrap_or(PdfObject::Null),
        other => other,
    }
}

fn parse_dest_array(
    dest_obj: &PdfObject,
    page_ids: &[ObjectId],
) -> Option<(usize, Option<f64>, Option<f64>, Option<f64>)> {
    let arr = dest_obj.as_array()?;
    let first = arr.first()?;

    let page_number = match first {
        PdfObject::Reference(page_ref) => page_ids
            .iter()
            .position(|id| id == page_ref)
            .map(|idx| idx + 1)?,
        PdfObject::Integer(page_idx) if *page_idx >= 0 && (*page_idx as usize) < page_ids.len() => {
            (*page_idx as usize) + 1
        }
        _ => return None,
    };

    let mode = arr.get(1).and_then(|o| o.as_name()).unwrap_or("Fit");
    match mode {
        "XYZ" => {
            let left = arr.get(2).and_then(|o| o.as_f64());
            let top = arr.get(3).and_then(|o| o.as_f64());
            let zoom = arr.get(4).and_then(|o| o.as_f64()).filter(|z| *z > 0.0);
            Some((page_number, left, top, zoom))
        }
        "FitH" | "FitBH" => {
            let top = arr.get(2).and_then(|o| o.as_f64());
            Some((page_number, None, top, None))
        }
        "FitV" | "FitBV" => {
            let left = arr.get(2).and_then(|o| o.as_f64());
            Some((page_number, left, None, None))
        }
        _ => Some((page_number, None, None, None)),
    }
}

/// Computes the number of descendants that would be visible if `item` itself were open.
fn potential_visible_descendants(item: &OutlineItem) -> usize {
    item.children
        .iter()
        .map(|child| {
            if child.is_open {
                1 + potential_visible_descendants(child)
            } else {
                1
            }
        })
        .sum()
}

/// Purges existing `/Outlines` objects from `doc` and removes `/Outlines` from `/Catalog`.
pub fn clear_outlines(doc: &mut PdfDocument) -> PdfResult<bool> {
    let Some(catalog_id) = doc.catalog_id() else {
        return Ok(false);
    };
    let mut cat_dict = doc.catalog()?;
    let Some(outlines_ref) = cat_dict.get("Outlines").and_then(|o| o.as_reference()) else {
        return Ok(false);
    };

    let mut to_remove = HashSet::new();
    collect_outline_object_ids(doc, outlines_ref, &mut to_remove, 0);

    for id in to_remove {
        doc.objects.remove(&id);
        doc.xref.entries.remove(&id);
    }

    cat_dict.remove("Outlines");
    doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));
    Ok(true)
}

fn collect_outline_object_ids(
    doc: &mut PdfDocument,
    node_id: ObjectId,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) {
    if depth > doc.limits.max_recursion_depth || !visited.insert(node_id) {
        return;
    }
    let Ok(obj) = doc.get_object(node_id) else {
        return;
    };
    let Some(dict) = obj.as_dict().cloned() else {
        return;
    };
    if let Some(first_id) = dict.get("First").and_then(|o| o.as_reference()) {
        collect_outline_object_ids(doc, first_id, visited, depth + 1);
    }
    if let Some(next_id) = dict.get("Next").and_then(|o| o.as_reference()) {
        collect_outline_object_ids(doc, next_id, visited, depth);
    }
}

/// Replaces the document's `/Outlines` hierarchy with `items`.
/// Returns the total number of outline items written across all levels.
pub fn set_outlines(doc: &mut PdfDocument, items: &[OutlineItem]) -> PdfResult<usize> {
    // Validate all items before mutating the document
    let page_ids = doc.get_pages()?;
    validate_outline_items(items, page_ids.len())?;

    clear_outlines(doc)?;
    if items.is_empty() {
        return Ok(0);
    }

    let catalog_id = doc.catalog_id().ok_or_else(|| PdfError::InvalidXRef {
        offset: 0,
        message: "Document trailer missing /Root reference".into(),
    })?;

    let root_id = reserve_object_id(doc);
    let (first_id, last_id, total_written) = write_outline_level(doc, root_id, items, &page_ids)?;

    let root_visible_count: i64 = items
        .iter()
        .map(|item| {
            let children_visible = if item.is_open {
                potential_visible_descendants(item)
            } else {
                0
            };
            (1 + children_visible) as i64
        })
        .sum();

    let mut root_dict = PdfDictionary::new();
    root_dict.insert("Type", PdfName::new("Outlines"));
    root_dict.insert("First", first_id);
    root_dict.insert("Last", last_id);
    root_dict.insert("Count", root_visible_count);
    doc.set_object(root_id, PdfObject::Dictionary(root_dict));

    let mut cat_dict = doc.catalog()?;
    cat_dict.insert("Outlines", root_id);
    doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));

    Ok(total_written)
}

fn validate_outline_items(items: &[OutlineItem], num_pages: usize) -> PdfResult<()> {
    for item in items {
        if item.title.trim().is_empty() {
            return Err(PdfError::OperationError(
                "Outline item title cannot be empty.".into(),
            ));
        }
        if let Some(page_num) = item.page_number {
            if page_num == 0 || page_num > num_pages {
                return Err(PdfError::OperationError(format!(
                    "Outline item '{}' targets page {}, but document has {} page(s).",
                    item.title, page_num, num_pages
                )));
            }
        }
        if let Some(ref uri) = item.uri {
            validate_outline_uri(uri)?;
        }
        validate_outline_items(&item.children, num_pages)?;
    }
    Ok(())
}

fn write_outline_level(
    doc: &mut PdfDocument,
    parent_id: ObjectId,
    items: &[OutlineItem],
    page_ids: &[ObjectId],
) -> PdfResult<(ObjectId, ObjectId, usize)> {
    let mut item_ids = Vec::with_capacity(items.len());
    for _ in 0..items.len() {
        item_ids.push(reserve_object_id(doc));
    }

    let mut total_written = items.len();

    for (idx, item) in items.iter().enumerate() {
        let this_id = item_ids[idx];
        let mut dict = PdfDictionary::new();
        dict.insert("Title", PdfObject::String(str_to_pdf_string(&item.title)));
        dict.insert("Parent", parent_id);

        if idx > 0 {
            dict.insert("Prev", item_ids[idx - 1]);
        }
        if idx + 1 < items.len() {
            dict.insert("Next", item_ids[idx + 1]);
        }

        // Style flags (/F)
        let mut flags = 0i64;
        if item.italic {
            flags |= 1;
        }
        if item.bold {
            flags |= 2;
        }
        if flags != 0 {
            dict.insert("F", flags);
        }

        // Color (/C)
        if let Some([r, g, b]) = item.color {
            let mut c_arr = PdfArray::new();
            c_arr.push(PdfObject::Real(r.clamp(0.0, 1.0)));
            c_arr.push(PdfObject::Real(g.clamp(0.0, 1.0)));
            c_arr.push(PdfObject::Real(b.clamp(0.0, 1.0)));
            dict.insert("C", PdfObject::Array(c_arr));
        }

        // Destination (/Dest) or URI Action (/A)
        if let Some(page_num) = item.page_number {
            if let Some(&page_id) = page_ids.get(page_num - 1) {
                let mut dest_arr = PdfArray::new();
                dest_arr.push(PdfObject::Reference(page_id));
                dest_arr.push(PdfObject::Name(PdfName::new("XYZ")));
                dest_arr.push(item.left.map(PdfObject::Real).unwrap_or(PdfObject::Null));
                dest_arr.push(item.top.map(PdfObject::Real).unwrap_or(PdfObject::Null));
                dest_arr.push(PdfObject::Real(item.zoom.unwrap_or(0.0)));
                dict.insert("Dest", PdfObject::Array(dest_arr));
            }
        } else if let Some(ref uri) = item.uri {
            let valid_uri = validate_outline_uri(uri)?;
            let mut action = PdfDictionary::new();
            action.insert("S", PdfName::new("URI"));
            action.insert(
                "URI",
                PdfObject::String(PdfString::literal(valid_uri.into_bytes())),
            );
            dict.insert("A", PdfObject::Dictionary(action));
        }

        // Children (/First, /Last, /Count)
        if !item.children.is_empty() {
            let (child_first, child_last, sub_written) =
                write_outline_level(doc, this_id, &item.children, page_ids)?;
            total_written += sub_written;

            let pot_vis = potential_visible_descendants(item) as i64;
            let count_entry = if item.is_open { pot_vis } else { -pot_vis };
            dict.insert("First", child_first);
            dict.insert("Last", child_last);
            dict.insert("Count", count_entry);
        }

        doc.set_object(this_id, PdfObject::Dictionary(dict));
    }

    let first_id = item_ids[0];
    let last_id = item_ids[item_ids.len() - 1];
    Ok((first_id, last_id, total_written))
}

/// Appends an `OutlineItem` to the existing outline tree at `parent_indices`
/// (empty slice appends at root level) and rewrites `/Outlines`.
pub fn add_outline_item(
    doc: &mut PdfDocument,
    item: OutlineItem,
    parent_indices: &[usize],
) -> PdfResult<usize> {
    let mut tree = extract_outlines(doc)?;
    if parent_indices.is_empty() {
        tree.push(item);
    } else {
        let mut target_list = &mut tree;
        for (depth, &idx) in parent_indices.iter().enumerate() {
            let node = target_list.get_mut(idx).ok_or_else(|| {
                PdfError::OperationError(format!(
                    "Invalid parent outline index {} at depth {}",
                    idx, depth
                ))
            })?;
            target_list = &mut node.children;
        }
        target_list.push(item);
    }
    set_outlines(doc, &tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_three_page_doc() -> PdfDocument {
        let mut doc = PdfDocument::empty();
        let mut page_ids = Vec::new();
        for _ in 0..3 {
            let pid = reserve_object_id(&mut doc);
            let mut page = PdfDictionary::new();
            page.insert("Type", PdfName::new("Page"));
            page.insert("Parent", ObjectId::new(2));
            doc.set_object(pid, PdfObject::Dictionary(page));
            page_ids.push(PdfObject::Reference(pid));
        }
        let mut pages_dict = PdfDictionary::new();
        pages_dict.insert("Type", PdfName::new("Pages"));
        pages_dict.insert("Kids", PdfObject::Array(page_ids));
        pages_dict.insert("Count", 3i64);
        doc.set_object(ObjectId::new(2), PdfObject::Dictionary(pages_dict));
        doc
    }

    #[test]
    fn test_set_and_extract_hierarchical_outlines_roundtrip() {
        let mut doc = create_three_page_doc();
        assert!(extract_outlines(&mut doc).unwrap().is_empty());

        let mut ch1 = OutlineItem::new_page("Capítulo 1: Introducción", 1);
        ch1.bold = true;
        ch1.top = Some(720.0);
        ch1.color = Some([0.1, 0.2, 0.8]);

        let mut sec1_1 = OutlineItem::new_page("1.1 Antecedentes", 1);
        sec1_1.top = Some(500.0);
        sec1_1.italic = true;

        let mut sec1_2 = OutlineItem::new_page("1.2 Alcance Técnico", 2);
        sec1_2.is_open = false;
        sec1_2
            .children
            .push(OutlineItem::new_page("1.2.1 Detalle A", 2));

        ch1.children.push(sec1_1);
        ch1.children.push(sec1_2);

        let mut ch2 = OutlineItem::new_page("Capítulo 2: Conclusiones", 3);
        ch2.children.push(OutlineItem::new_uri(
            "Anexo Web Oficial",
            "https://example.com/anexo",
        ));

        let tree = vec![ch1, ch2];
        let written = set_outlines(&mut doc, &tree).unwrap();
        assert_eq!(written, 6);

        let bytes = doc.save_to_vec().unwrap();
        let mut reloaded = PdfDocument::load(&bytes).unwrap();
        let extracted = extract_outlines(&mut reloaded).unwrap();
        assert_eq!(extracted.len(), 2);

        assert_eq!(extracted[0].title, "Capítulo 1: Introducción");
        assert_eq!(extracted[0].page_number, Some(1));
        assert_eq!(extracted[0].top, Some(720.0));
        assert!(extracted[0].bold);
        assert!(!extracted[0].italic);
        assert!(extracted[0].is_open);
        assert_eq!(extracted[0].children.len(), 2);

        assert_eq!(extracted[0].children[0].title, "1.1 Antecedentes");
        assert!(extracted[0].children[0].italic);
        assert_eq!(extracted[0].children[1].title, "1.2 Alcance Técnico");
        assert!(!extracted[0].children[1].is_open);
        assert_eq!(extracted[0].children[1].children.len(), 1);
        assert_eq!(
            extracted[0].children[1].children[0].title,
            "1.2.1 Detalle A"
        );
        assert_eq!(extracted[0].children[1].children[0].page_number, Some(2));

        assert_eq!(extracted[1].title, "Capítulo 2: Conclusiones");
        assert_eq!(extracted[1].page_number, Some(3));
        assert_eq!(extracted[1].children.len(), 1);
        assert_eq!(
            extracted[1].children[0].uri.as_deref(),
            Some("https://example.com/anexo")
        );

        // Clear outlines
        assert!(clear_outlines(&mut reloaded).unwrap());
        assert!(extract_outlines(&mut reloaded).unwrap().is_empty());
    }

    #[test]
    fn test_outline_rejects_invalid_page_and_unsafe_uri() {
        let mut doc = create_three_page_doc();
        let bad_page = vec![OutlineItem::new_page("Out of range", 99)];
        assert!(set_outlines(&mut doc, &bad_page).is_err());

        let unsafe_uri = vec![OutlineItem::new_uri("XSS", "javascript:alert(1)")];
        assert!(set_outlines(&mut doc, &unsafe_uri).is_err());
    }
}
