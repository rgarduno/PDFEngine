//! Removes active-content actions before a document is written.
//!
//! ISO 32000 allows `/JavaScript`, `/Launch`, and `/SubmitForm` actions, plus
//! `/OpenAction` and `/AA` hooks that run them. This pass deletes those actions
//! from the object graph. A `/URI` action is kept when its scheme is not one
//! that runs code or reads a local file.

use std::collections::HashSet;

use crate::cos::{ObjectId, PdfDictionary, PdfDocument, PdfObject, XRefEntry};
use crate::error::PdfResult;

const DROPPED_KEYS: &[&str] = &[
    "OpenAction",
    "AA",
    "JavaScript",
    "JS",
    "Launch",
    "SubmitForm",
];

/// Keys whose target is part of the removed action and must not be rewritten.
const PAYLOAD_KEYS: &[&str] = &[
    "JS",
    "JavaScript",
    "F",
    "Win",
    "Unix",
    "Mac",
    "Params",
    "EF",
    "DOS",
];

const DANGEROUS_SCHEMES: &[&str] = &[
    "javascript",
    "jscript",
    "livescript",
    "vbscript",
    "file",
    "data",
];

/// Deletes active-content actions from `doc` so a later save cannot emit them.
///
/// Object streams that were fully expanded are dropped as well. Their compressed
/// bytes would otherwise keep a second copy of the actions.
pub(crate) fn neutralize_active_content(doc: &mut PdfDocument) -> PdfResult<()> {
    materialize(doc);
    let check_uri = !doc.xref.trailer.contains_key("Encrypt");
    let payload = harvest(doc, check_uri);
    discard_expanded_object_streams(doc);

    let depth_limit = doc.limits.max_recursion_depth;
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let protected = doc.objects.get(&id).is_some_and(is_protected);
        if payload.contains(&id) && !protected {
            doc.objects.insert(id, PdfObject::Null);
            continue;
        }
        if let Some(obj) = doc.objects.get_mut(&id) {
            scrub_object(obj, &payload, check_uri, 0, depth_limit);
        }
    }
    scrub_dict(&mut doc.xref.trailer, &payload, check_uri, 0, depth_limit);
    Ok(())
}

fn materialize(doc: &mut PdfDocument) {
    let ids: Vec<ObjectId> = doc
        .xref
        .entries
        .iter()
        .filter_map(|(&id, entry)| match entry {
            XRefEntry::InUse { .. } | XRefEntry::Compressed { .. } => Some(id),
            XRefEntry::Free { .. } => None,
        })
        .collect();
    for id in ids {
        let _ = doc.get_object(id);
    }
}

fn discard_expanded_object_streams(doc: &mut PdfDocument) {
    let containers: Vec<u32> = doc
        .xref
        .entries
        .values()
        .filter_map(|entry| match entry {
            XRefEntry::Compressed { container_id, .. } => Some(*container_id),
            _ => None,
        })
        .collect();

    let mut seen = HashSet::new();
    for container_number in containers {
        if !seen.insert(container_number) {
            continue;
        }
        let children: Vec<ObjectId> = doc
            .xref
            .entries
            .iter()
            .filter_map(|(&id, entry)| match entry {
                XRefEntry::Compressed { container_id, .. } if *container_id == container_number => {
                    Some(id)
                }
                _ => None,
            })
            .collect();
        if children.is_empty() || !children.iter().all(|id| doc.objects.contains_key(id)) {
            continue;
        }
        let container_ids: Vec<ObjectId> = doc
            .xref
            .entries
            .keys()
            .copied()
            .filter(|id| id.number == container_number)
            .collect();
        for id in container_ids {
            doc.objects.remove(&id);
            doc.xref.entries.remove(&id);
        }
    }
}

fn harvest(doc: &PdfDocument, check_uri: bool) -> HashSet<ObjectId> {
    let limit = doc.limits.max_recursion_depth;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let Some(obj) = doc.objects.get(&id).cloned() else {
            continue;
        };
        if !is_protected(&obj) && object_is_dangerous(&obj, check_uri) && seen.insert(id) {
            out.push(id);
            harvest_payload(&obj, doc, check_uri, &mut seen, &mut out, 0, limit);
        }
        harvest_inline(&obj, doc, check_uri, &mut seen, &mut out, 0, limit);
    }
    out.into_iter().collect()
}

fn harvest_inline(
    obj: &PdfObject,
    doc: &PdfDocument,
    check_uri: bool,
    seen: &mut HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
    depth: usize,
    limit: usize,
) {
    if depth > limit {
        return;
    }
    match obj {
        PdfObject::Dictionary(dict) => {
            harvest_dict(dict, doc, check_uri, seen, out, depth, limit);
        }
        PdfObject::Stream(stream) => {
            harvest_dict(&stream.dict, doc, check_uri, seen, out, depth, limit);
        }
        PdfObject::Array(items) => {
            for item in items {
                harvest_inline(item, doc, check_uri, seen, out, depth + 1, limit);
            }
        }
        _ => {}
    }
}

fn harvest_dict(
    dict: &PdfDictionary,
    doc: &PdfDocument,
    check_uri: bool,
    seen: &mut HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
    depth: usize,
    limit: usize,
) {
    for key in DROPPED_KEYS {
        if let Some(value) = dict.get(key) {
            match value {
                PdfObject::Reference(id) => {
                    push_target(*id, doc, check_uri, seen, out, depth + 1, limit);
                }
                other => harvest_inline(other, doc, check_uri, seen, out, depth + 1, limit),
            }
        }
    }
    for (key, value) in dict.iter() {
        if DROPPED_KEYS.contains(&key.as_str()) {
            continue;
        }
        match value {
            PdfObject::Dictionary(child) if is_dangerous_action(child, check_uri) => {
                harvest_payload_dict(child, doc, check_uri, seen, out, depth + 1, limit);
            }
            PdfObject::Reference(id) => {
                let dangerous = doc
                    .objects
                    .get(id)
                    .is_some_and(|target| object_is_dangerous(target, check_uri));
                if dangerous {
                    push_target(*id, doc, check_uri, seen, out, depth + 1, limit);
                }
            }
            other => harvest_inline(other, doc, check_uri, seen, out, depth + 1, limit),
        }
    }
}

fn harvest_payload(
    obj: &PdfObject,
    doc: &PdfDocument,
    check_uri: bool,
    seen: &mut HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
    depth: usize,
    limit: usize,
) {
    match obj {
        PdfObject::Dictionary(dict) => {
            harvest_payload_dict(dict, doc, check_uri, seen, out, depth, limit);
        }
        PdfObject::Stream(stream) => {
            harvest_payload_dict(&stream.dict, doc, check_uri, seen, out, depth, limit);
        }
        _ => {}
    }
}

fn harvest_payload_dict(
    dict: &PdfDictionary,
    doc: &PdfDocument,
    check_uri: bool,
    seen: &mut HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
    depth: usize,
    limit: usize,
) {
    for key in PAYLOAD_KEYS {
        let Some(value) = dict.get(key) else {
            continue;
        };
        match value {
            PdfObject::Reference(id) => {
                push_target(*id, doc, check_uri, seen, out, depth + 1, limit);
            }
            other => harvest_inline(other, doc, check_uri, seen, out, depth + 1, limit),
        }
    }
}

fn push_target(
    id: ObjectId,
    doc: &PdfDocument,
    check_uri: bool,
    seen: &mut HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
    depth: usize,
    limit: usize,
) {
    if depth > limit || !seen.insert(id) {
        return;
    }
    let Some(obj) = doc.objects.get(&id).cloned() else {
        out.push(id);
        return;
    };
    if is_protected(&obj) {
        return;
    }
    out.push(id);
    harvest_payload(&obj, doc, check_uri, seen, out, depth, limit);
}

fn scrub_object(
    obj: &mut PdfObject,
    payload: &HashSet<ObjectId>,
    check_uri: bool,
    depth: usize,
    limit: usize,
) {
    if depth > limit {
        *obj = PdfObject::Null;
        return;
    }
    match obj {
        PdfObject::Dictionary(dict) => {
            if is_dangerous_action(dict, check_uri) {
                *obj = PdfObject::Null;
                return;
            }
            scrub_dict(dict, payload, check_uri, depth, limit);
        }
        PdfObject::Stream(stream) => {
            if is_dangerous_action(&stream.dict, check_uri) {
                *obj = PdfObject::Null;
                return;
            }
            scrub_dict(&mut stream.dict, payload, check_uri, depth, limit);
        }
        PdfObject::Array(items) => {
            for item in items.iter_mut() {
                if value_is_dropped(item, payload, check_uri) {
                    *item = PdfObject::Null;
                } else {
                    scrub_object(item, payload, check_uri, depth + 1, limit);
                }
            }
        }
        _ => {}
    }
}

fn scrub_dict(
    dict: &mut PdfDictionary,
    payload: &HashSet<ObjectId>,
    check_uri: bool,
    depth: usize,
    limit: usize,
) {
    for key in DROPPED_KEYS {
        dict.remove(*key);
    }
    // `/S` sits on the action itself. A page or annotation must lose that name
    // without the whole dictionary being replaced.
    if let Some(subtype) = dict.get("S").and_then(|value| value.as_name()) {
        let dangerous_subtype = matches!(subtype, "JavaScript" | "Launch" | "SubmitForm")
            || (check_uri
                && subtype == "URI"
                && dict.get("URI").is_some_and(uri_value_is_dangerous));
        if dangerous_subtype {
            for key in [
                "S",
                "JS",
                "URI",
                "F",
                "Win",
                "Unix",
                "Mac",
                "Params",
                "JavaScript",
                "Launch",
                "SubmitForm",
            ] {
                dict.remove(key);
            }
        }
    }
    let keys: Vec<String> = dict
        .iter()
        .map(|(key, _)| key.as_str().to_string())
        .collect();
    for key in keys {
        let drop_value = dict
            .get(&key)
            .is_some_and(|value| value_is_dropped(value, payload, check_uri));
        if drop_value {
            dict.remove(&key);
            continue;
        }
        if let Some(value) = dict.get_mut(&key) {
            scrub_object(value, payload, check_uri, depth + 1, limit);
        }
    }
}

fn value_is_dropped(value: &PdfObject, payload: &HashSet<ObjectId>, check_uri: bool) -> bool {
    match value {
        PdfObject::Reference(id) => payload.contains(id),
        PdfObject::Dictionary(dict) => is_dangerous_action(dict, check_uri),
        PdfObject::Stream(stream) => is_dangerous_action(&stream.dict, check_uri),
        _ => false,
    }
}

fn object_is_dangerous(obj: &PdfObject, check_uri: bool) -> bool {
    match obj {
        PdfObject::Dictionary(dict) => is_dangerous_action(dict, check_uri),
        PdfObject::Stream(stream) => is_dangerous_action(&stream.dict, check_uri),
        _ => false,
    }
}

fn is_dangerous_action(dict: &PdfDictionary, check_uri: bool) -> bool {
    if is_protected_dict(dict) {
        return false;
    }
    let Some(subtype) = dict.get("S").and_then(|value| value.as_name()) else {
        return false;
    };
    if matches!(subtype, "JavaScript" | "Launch" | "SubmitForm") {
        return true;
    }
    if check_uri && subtype == "URI" {
        if let Some(uri) = dict.get("URI") {
            return uri_value_is_dangerous(uri);
        }
    }
    false
}

fn uri_value_is_dangerous(value: &PdfObject) -> bool {
    match value {
        PdfObject::String(text) => scheme_is_dangerous(&text.bytes),
        _ => false,
    }
}

fn scheme_is_dangerous(bytes: &[u8]) -> bool {
    let text = decode_pdf_text(bytes);
    let trimmed = text.trim_start_matches(|ch: char| ch.is_whitespace() || ch.is_control());
    if !trimmed.contains(':') {
        return false;
    }
    let scheme: String = trimmed
        .chars()
        .take_while(|ch| *ch != ':')
        .filter(|ch| !ch.is_whitespace() && !ch.is_control())
        .collect();
    DANGEROUS_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str())
}

fn decode_pdf_text(bytes: &[u8]) -> String {
    if let Some(units) = utf16_units(bytes) {
        return String::from_utf16_lossy(&units);
    }
    bytes.iter().map(|byte| *byte as char).collect()
}

fn utf16_units(bytes: &[u8]) -> Option<Vec<u16>> {
    let rest = if bytes.starts_with(&[0xFE, 0xFF]) {
        Some((&bytes[2..], true))
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some((&bytes[2..], false))
    } else {
        None
    }?;
    let (body, big_endian) = rest;
    Some(
        body.chunks_exact(2)
            .map(|pair| {
                if big_endian {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect(),
    )
}

fn is_protected(obj: &PdfObject) -> bool {
    obj.as_dict().is_some_and(is_protected_dict)
}

fn is_protected_dict(dict: &PdfDictionary) -> bool {
    matches!(
        dict.get("Type").and_then(|value| value.as_name()),
        Some(
            "Catalog"
                | "Pages"
                | "Page"
                | "Font"
                | "FontDescriptor"
                | "XObject"
                | "ObjStm"
                | "XRef"
                | "Metadata"
                | "Outlines"
                | "Annot"
                | "Pattern"
                | "ExtGState"
                | "Encoding"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::{PdfName, PdfStream, PdfString};

    /// `alloc_object_id` reads the current maximum and does not reserve it.
    fn reserve(doc: &mut PdfDocument) -> ObjectId {
        let id = doc.alloc_object_id();
        doc.set_object(id, PdfObject::Null);
        id
    }

    fn document_with_active_content() -> PdfDocument {
        let mut doc = PdfDocument::empty();
        let page_id = reserve(&mut doc);
        let script_id = reserve(&mut doc);
        let action_id = reserve(&mut doc);
        let aa_id = reserve(&mut doc);
        let bad_link_id = reserve(&mut doc);
        let good_link_id = reserve(&mut doc);
        let names_id = reserve(&mut doc);
        let submit_id = reserve(&mut doc);
        let file_link_id = reserve(&mut doc);

        let mut script_dict = PdfDictionary::new();
        script_dict.insert("Length", PdfObject::Integer(b"hidden-script".len() as i64));
        doc.set_object(
            script_id,
            PdfObject::Stream(PdfStream::new(script_dict, b"hidden-script".to_vec())),
        );

        let mut action = PdfDictionary::new();
        action.insert("S", PdfObject::Name(PdfName::new("JavaScript")));
        action.insert("JS", PdfObject::Reference(script_id));
        doc.set_object(action_id, PdfObject::Dictionary(action));

        let mut launch = PdfDictionary::new();
        launch.insert("S", PdfObject::Name(PdfName::new("Launch")));
        launch.insert(
            "F",
            PdfObject::String(PdfString::literal(b"calc.exe".to_vec())),
        );
        let mut aa = PdfDictionary::new();
        aa.insert("O", PdfObject::Dictionary(launch));
        doc.set_object(aa_id, PdfObject::Dictionary(aa));

        let mut bad_action = PdfDictionary::new();
        bad_action.insert("S", PdfObject::Name(PdfName::new("URI")));
        bad_action.insert(
            "URI",
            PdfObject::String(PdfString::literal(b"javascript:alert(1)".to_vec())),
        );
        let mut bad_link = PdfDictionary::new();
        bad_link.insert("Type", PdfObject::Name(PdfName::new("Annot")));
        bad_link.insert("Subtype", PdfObject::Name(PdfName::new("Link")));
        bad_link.insert("A", PdfObject::Dictionary(bad_action));
        doc.set_object(bad_link_id, PdfObject::Dictionary(bad_link));

        let mut file_action = PdfDictionary::new();
        file_action.insert("S", PdfObject::Name(PdfName::new("URI")));
        file_action.insert(
            "URI",
            PdfObject::String(PdfString::literal(b"file:///tmp/secret".to_vec())),
        );
        let mut file_link = PdfDictionary::new();
        file_link.insert("Type", PdfObject::Name(PdfName::new("Annot")));
        file_link.insert("Subtype", PdfObject::Name(PdfName::new("Link")));
        file_link.insert("A", PdfObject::Dictionary(file_action));
        doc.set_object(file_link_id, PdfObject::Dictionary(file_link));

        let mut good_action = PdfDictionary::new();
        good_action.insert("S", PdfObject::Name(PdfName::new("URI")));
        good_action.insert(
            "URI",
            PdfObject::String(PdfString::literal(b"https://example.com/docs".to_vec())),
        );
        let mut good_link = PdfDictionary::new();
        good_link.insert("Type", PdfObject::Name(PdfName::new("Annot")));
        good_link.insert("Subtype", PdfObject::Name(PdfName::new("Link")));
        good_link.insert("A", PdfObject::Dictionary(good_action));
        doc.set_object(good_link_id, PdfObject::Dictionary(good_link));

        let mut submit = PdfDictionary::new();
        submit.insert("S", PdfObject::Name(PdfName::new("SubmitForm")));
        submit.insert(
            "F",
            PdfObject::String(PdfString::literal(b"http://evil.example/collect".to_vec())),
        );
        doc.set_object(submit_id, PdfObject::Dictionary(submit));

        let mut js_leaf = PdfDictionary::new();
        js_leaf.insert("S", PdfObject::Name(PdfName::new("JavaScript")));
        js_leaf.insert(
            "JS",
            PdfObject::String(PdfString::literal(b"app.alert(1)".to_vec())),
        );
        let mut js_tree = PdfDictionary::new();
        js_tree.insert(
            "Names",
            PdfObject::Array(vec![
                PdfObject::String(PdfString::literal(b"click".to_vec())),
                PdfObject::Dictionary(js_leaf),
            ]),
        );
        let mut names = PdfDictionary::new();
        names.insert("JavaScript", PdfObject::Dictionary(js_tree));
        names.insert("Dests", PdfObject::Dictionary(PdfDictionary::new()));
        doc.set_object(names_id, PdfObject::Dictionary(names));

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfObject::Name(PdfName::new("Page")));
        page.insert("Parent", PdfObject::Reference(ObjectId::new(2)));
        page.insert("AA", PdfObject::Reference(aa_id));
        page.insert(
            "Annots",
            PdfObject::Array(vec![
                PdfObject::Reference(bad_link_id),
                PdfObject::Reference(file_link_id),
                PdfObject::Reference(good_link_id),
                PdfObject::Reference(submit_id),
            ]),
        );
        doc.set_object(page_id, PdfObject::Dictionary(page));

        let pages = doc
            .objects
            .get_mut(&ObjectId::new(2))
            .and_then(|obj| obj.as_dict_mut())
            .expect("pages");
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference(page_id)]),
        );
        pages.insert("Count", PdfObject::Integer(1));

        let catalog = doc
            .objects
            .get_mut(&ObjectId::new(1))
            .and_then(|obj| obj.as_dict_mut())
            .expect("catalog");
        catalog.insert("OpenAction", PdfObject::Reference(action_id));
        catalog.insert("Names", PdfObject::Reference(names_id));
        doc
    }

    #[test]
    fn test_save_strips_active_actions_and_keeps_https_uri() {
        let mut doc = document_with_active_content();
        let saved = doc.save_to_vec().unwrap();

        for marker in [
            b"/JavaScript".as_slice(),
            b"/JS".as_slice(),
            b"/Launch".as_slice(),
            b"/SubmitForm".as_slice(),
            b"/OpenAction".as_slice(),
            b"/AA".as_slice(),
            b"javascript:".as_slice(),
            b"file:///tmp/secret".as_slice(),
            b"hidden-script".as_slice(),
            b"app.alert(1)".as_slice(),
            b"calc.exe".as_slice(),
            b"evil.example".as_slice(),
        ] {
            assert!(
                !saved.windows(marker.len()).any(|window| window == marker),
                "output still contains {}",
                String::from_utf8_lossy(marker)
            );
        }
        assert!(saved
            .windows(b"https://example.com/docs".len())
            .any(|window| window == b"https://example.com/docs"));

        let mut reloaded = PdfDocument::load(&saved).unwrap();
        let catalog = reloaded.catalog().unwrap();
        assert!(!catalog.contains_key("OpenAction"));
        assert!(catalog.contains_key("Pages"));
        let pages = reloaded.get_pages().unwrap();
        assert_eq!(pages.len(), 1);
        let page = reloaded.get_object(pages[0]).unwrap();
        let page_dict = page.as_dict().unwrap();
        assert!(!page_dict.contains_key("AA"));
        assert_eq!(
            page_dict.get("Type").and_then(|value| value.as_name()),
            Some("Page")
        );
    }

    #[test]
    fn test_utf16_javascript_scheme_is_removed() {
        let mut doc = PdfDocument::empty();
        let mut encoded = vec![0xFE, 0xFF];
        for unit in "javascript:alert(1)".encode_utf16() {
            encoded.extend_from_slice(&unit.to_be_bytes());
        }
        let mut action = PdfDictionary::new();
        action.insert("S", PdfObject::Name(PdfName::new("URI")));
        action.insert("URI", PdfObject::String(PdfString::literal(encoded)));
        let mut link = PdfDictionary::new();
        link.insert("Type", PdfObject::Name(PdfName::new("Annot")));
        link.insert("Subtype", PdfObject::Name(PdfName::new("Link")));
        link.insert("A", PdfObject::Dictionary(action));
        let link_id = doc.alloc_object_id();
        doc.set_object(link_id, PdfObject::Dictionary(link));

        let saved = doc.save_to_vec().unwrap();
        assert!(!saved
            .windows(b"javascript:".len())
            .any(|window| window == b"javascript:"));
        assert!(!saved.windows(b"/URI".len()).any(|window| window == b"/URI"));
        let mut reloaded = PdfDocument::load(&saved).unwrap();
        let annot = reloaded.get_object(link_id).unwrap();
        let annot_dict = annot.as_dict().unwrap();
        assert!(!annot_dict.contains_key("A"));
        assert_eq!(
            annot_dict.get("Subtype").and_then(|value| value.as_name()),
            Some("Link")
        );
    }
}
