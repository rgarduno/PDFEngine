//! Clears structure-tree text that repeats text a redaction removed.
//!
//! `/ActualText`, `/Alt`, `/E`, and `/T` are removed when the decoded string
//! repeats a removed string of at least 4 Unicode scalars. The element stays.
//! `/S`, `/P`, `/K`, `/Pg`, `/ID`, `/Lang`, `/A`, the role map, and parent-tree
//! entries stay. The walk follows `/K` and, from the structure root, the parent
//! number tree (`/Nums` values and `/Kids`). It does not follow `/P`, `/Pg`,
//! `/Obj`, or the role map. Number-tree keys are not object ids. No removed
//! text leaves the tree unchanged. A match on one page clears that text in the
//! whole tree. The whole entry is removed. A value that is not a PDF string
//! stays. An inline dictionary is cleared when it sits in a dictionary this
//! pass writes back. A tree deeper than 64 is not fully cleared. Content
//! streams, images, forms, and appearances are not walked again.

use std::collections::HashSet;

use crate::cos::object::{ObjectId, PdfDictionary, PdfObject};
use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::redact::embedded::{self, MIN_NEEDLE_SCALARS};

const MAX_STRUCT_DEPTH: usize = 64;
const TEXT_KEYS: [&str; 4] = ["ActualText", "Alt", "E", "T"];

struct Scrub<'a> {
    doc: &'a mut PdfDocument,
    exact: &'a [&'a str],
    ignore_case: Option<&'a str>,
    seen: &'a mut HashSet<ObjectId>,
    drop_ids: &'a mut Vec<ObjectId>,
    cleared: &'a mut usize,
}

/// Removes structure-tree text that repeats `exact` or, when set, `ignore_case`.
///
/// `ignore_case` is the original text query. It is used only for a
/// case-insensitive text redaction. Returns how many entries were removed.
pub(super) fn clear_structure_text_repeating(
    doc: &mut PdfDocument,
    exact: &[String],
    ignore_case: Option<&str>,
) -> PdfResult<usize> {
    let exact_refs: Vec<&str> = exact
        .iter()
        .map(String::as_str)
        .filter(|needle| needle.chars().count() >= MIN_NEEDLE_SCALARS)
        .collect();
    let ignore_case = ignore_case.filter(|query| query.chars().count() >= MIN_NEEDLE_SCALARS);
    if exact_refs.is_empty() && ignore_case.is_none() {
        return Ok(0);
    }

    let Some(catalog_id) = doc.catalog_id() else {
        return Ok(0);
    };
    let mut catalog = match doc.get_object(catalog_id) {
        Ok(PdfObject::Dictionary(dict)) => dict,
        Ok(_) => return Ok(0),
        Err(err) => return Err(err),
    };
    let Some(root) = catalog.get("StructTreeRoot").cloned() else {
        return Ok(0);
    };

    let mut seen = HashSet::new();
    let mut drop_ids = Vec::new();
    let mut cleared = 0usize;
    {
        let mut scrub = Scrub {
            doc,
            exact: &exact_refs,
            ignore_case,
            seen: &mut seen,
            drop_ids: &mut drop_ids,
            cleared: &mut cleared,
        };
        match root {
            PdfObject::Reference(id) => {
                if scrub.doc.get_object(id).is_err() {
                    return Ok(0);
                }
                scrub.walk_struct_id(id, 0, true);
            }
            PdfObject::Dictionary(mut dict) => {
                if scrub.scrub_struct_dict(&mut dict, 0, true) {
                    catalog.insert("StructTreeRoot", PdfObject::Dictionary(dict));
                    scrub
                        .doc
                        .set_object(catalog_id, PdfObject::Dictionary(catalog));
                }
            }
            _ => return Ok(0),
        }
        scrub.release_unreferenced();
    }
    Ok(cleared)
}

impl Scrub<'_> {
    fn walk_struct_id(&mut self, id: ObjectId, depth: usize, is_root: bool) {
        if depth > MAX_STRUCT_DEPTH || !self.seen.insert(id) {
            return;
        }
        let object = match self.doc.get_object(id) {
            Ok(object) => object,
            Err(_) => return,
        };
        match object {
            PdfObject::Dictionary(mut dict) => {
                if self.scrub_struct_dict(&mut dict, depth, is_root) {
                    self.doc.set_object(id, PdfObject::Dictionary(dict));
                }
            }
            PdfObject::Array(mut items) => {
                let mut changed = false;
                for item in &mut items {
                    changed |= self.walk_struct_value(item, depth);
                }
                if changed {
                    self.doc.set_object(id, PdfObject::Array(items));
                }
            }
            _ => {}
        }
    }

    fn scrub_struct_dict(&mut self, dict: &mut PdfDictionary, depth: usize, is_root: bool) -> bool {
        let mut changed = self.clear_text_keys(dict);
        if let Some(kid) = dict.get_mut("K") {
            changed |= self.walk_struct_value(kid, depth + 1);
        }
        if is_root {
            if let Some(parent) = dict.get_mut("ParentTree") {
                changed |= self.walk_number_node_value(parent, depth + 1);
            }
        }
        changed
    }

    fn walk_struct_value(&mut self, value: &mut PdfObject, depth: usize) -> bool {
        if depth > MAX_STRUCT_DEPTH {
            return false;
        }
        match value {
            PdfObject::Reference(id) => {
                self.walk_struct_id(*id, depth, false);
                false
            }
            PdfObject::Dictionary(dict) => self.scrub_struct_dict(dict, depth, false),
            PdfObject::Array(items) => {
                let mut changed = false;
                for item in items {
                    changed |= self.walk_struct_value(item, depth);
                }
                changed
            }
            _ => false,
        }
    }

    fn walk_number_node_value(&mut self, value: &mut PdfObject, depth: usize) -> bool {
        if depth > MAX_STRUCT_DEPTH {
            return false;
        }
        match value {
            PdfObject::Reference(id) => {
                self.walk_number_id(*id, depth);
                false
            }
            PdfObject::Dictionary(dict) => self.scrub_number_dict(dict, depth),
            _ => false,
        }
    }

    fn walk_number_id(&mut self, id: ObjectId, depth: usize) {
        if depth > MAX_STRUCT_DEPTH || !self.seen.insert(id) {
            return;
        }
        let Ok(PdfObject::Dictionary(mut dict)) = self.doc.get_object(id) else {
            return;
        };
        if self.scrub_number_dict(&mut dict, depth) {
            self.doc.set_object(id, PdfObject::Dictionary(dict));
        }
    }

    fn scrub_number_dict(&mut self, dict: &mut PdfDictionary, depth: usize) -> bool {
        let mut changed = self.clear_text_keys(dict);
        if let Some(nums) = dict.get_mut("Nums") {
            changed |= self.walk_nums_container(nums, depth + 1);
        }
        if let Some(kids) = dict.get_mut("Kids") {
            changed |= self.walk_kids_container(kids, depth + 1);
        }
        changed
    }

    fn walk_nums_container(&mut self, value: &mut PdfObject, depth: usize) -> bool {
        if depth > MAX_STRUCT_DEPTH {
            return false;
        }
        match value {
            PdfObject::Array(items) => self.scrub_pairs(items, depth),
            PdfObject::Reference(id) => {
                let id = *id;
                if !self.seen.insert(id) {
                    return false;
                }
                let Ok(PdfObject::Array(mut items)) = self.doc.get_object(id) else {
                    self.seen.remove(&id);
                    self.walk_struct_id(id, depth, false);
                    return false;
                };
                let changed = self.scrub_pairs(&mut items, depth);
                if changed {
                    self.doc.set_object(id, PdfObject::Array(items));
                }
                false
            }
            _ => false,
        }
    }

    /// Walks number-tree values. Integer keys are not object ids.
    fn scrub_pairs(&mut self, items: &mut [PdfObject], depth: usize) -> bool {
        let mut changed = false;
        let mut index = 0;
        while index + 1 < items.len() {
            changed |= self.walk_struct_value(&mut items[index + 1], depth);
            index += 2;
        }
        changed
    }

    fn walk_kids_container(&mut self, value: &mut PdfObject, depth: usize) -> bool {
        if depth > MAX_STRUCT_DEPTH {
            return false;
        }
        match value {
            PdfObject::Array(items) => {
                let mut changed = false;
                for item in items {
                    changed |= self.walk_number_node_value(item, depth);
                }
                changed
            }
            PdfObject::Reference(id) => {
                let id = *id;
                if !self.seen.insert(id) {
                    return false;
                }
                let Ok(PdfObject::Array(mut items)) = self.doc.get_object(id) else {
                    self.seen.remove(&id);
                    return false;
                };
                let mut changed = false;
                for item in &mut items {
                    changed |= self.walk_number_node_value(item, depth);
                }
                if changed {
                    self.doc.set_object(id, PdfObject::Array(items));
                }
                false
            }
            _ => false,
        }
    }

    fn clear_text_keys(&mut self, dict: &mut PdfDictionary) -> bool {
        let mut changed = false;
        for key in TEXT_KEYS {
            let matches = dict
                .get(key)
                .is_some_and(|value| self.text_matches(value));
            if !matches {
                continue;
            }
            if let Some(PdfObject::Reference(id)) = dict.remove(key) {
                self.drop_ids.push(id);
            }
            *self.cleared += 1;
            changed = true;
        }
        changed
    }

    fn text_matches(&mut self, value: &PdfObject) -> bool {
        let Some(text) = self.text_of(value) else {
            return false;
        };
        embedded::text_repeats(&text, self.exact, self.ignore_case)
    }

    fn text_of(&mut self, value: &PdfObject) -> Option<String> {
        match value {
            PdfObject::String(text) => Some(text.to_string_lossy()),
            PdfObject::Reference(id) => match self.doc.get_object(*id) {
                Ok(PdfObject::String(text)) => Some(text.to_string_lossy()),
                _ => None,
            },
            _ => None,
        }
    }

    fn release_unreferenced(&mut self) {
        let mut ids = std::mem::take(self.drop_ids);
        ids.sort();
        ids.dedup();
        for id in ids {
            let is_text = matches!(
                self.doc.objects.get(&id),
                Some(PdfObject::String(_) | PdfObject::Name(_))
            );
            if is_text && !referenced(self.doc, id) {
                self.doc.objects.remove(&id);
                self.doc.xref.entries.remove(&id);
            }
        }
    }
}

fn referenced(doc: &PdfDocument, target: ObjectId) -> bool {
    doc.objects
        .iter()
        .any(|(id, object)| *id != target && object_references(object, target))
}

fn object_references(object: &PdfObject, target: ObjectId) -> bool {
    match object {
        PdfObject::Reference(id) => *id == target,
        PdfObject::Array(items) => items.iter().any(|item| object_references(item, target)),
        PdfObject::Dictionary(dict) => dict
            .iter()
            .any(|(_, value)| object_references(value, target)),
        PdfObject::Stream(stream) => stream
            .dict
            .iter()
            .any(|(_, value)| object_references(value, target)),
        _ => false,
    }
}
