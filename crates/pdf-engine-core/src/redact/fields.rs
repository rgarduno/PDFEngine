//! Clears a form field value and appearance when they repeat removed text.
//!
//! The field dictionary and its widget stay in `/Fields` and `/Annots`. `/V`
//! and `/AP` are removed together when either one repeats a removed string of
//! at least 4 characters. The option list, the default value, and the field
//! name stay. An appearance whose filter cannot be decoded does not match on
//! its own. An image inside that appearance is left for a later pass.

use std::collections::HashSet;

use crate::cos::decode_stream;
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream};
use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::layout::reconstructor::LayoutReconstructor;
use crate::redact::embedded::{self, MIN_NEEDLE_SCALARS};
use crate::stream::parser::{build_ast_from_operations, ContentStreamTokenizer};

const MAX_FIELD_DEPTH: usize = 32;
const MAX_VALUE_DEPTH: usize = 4;

struct FieldNode {
    id: ObjectId,
    widgets: Vec<ObjectId>,
}

struct AppearanceHit {
    streams: Vec<PdfStream>,
    stream_ids: Vec<ObjectId>,
    dict_ids: Vec<ObjectId>,
}

/// Removes `/V` and `/AP` from fields that repeat `exact` or, when set, `ignore_case`.
///
/// `ignore_case` is the original text query. It is used only for a
/// case-insensitive text redaction. A match on one page clears that field in
/// the whole document. Returns how many fields were cleared.
pub(super) fn clear_form_values_repeating(
    doc: &mut PdfDocument,
    exact: &[String],
    ignore_case: Option<&str>,
) -> PdfResult<usize> {
    let exact: Vec<&str> = exact
        .iter()
        .map(String::as_str)
        .filter(|needle| needle.chars().count() >= MIN_NEEDLE_SCALARS)
        .collect();
    let ignore_case = ignore_case.filter(|query| query.chars().count() >= MIN_NEEDLE_SCALARS);
    if exact.is_empty() && ignore_case.is_none() {
        return Ok(0);
    }

    let nodes = collect_fields(doc)?;
    let mut cleared = 0usize;
    for node in &nodes {
        if !node_repeats(doc, node, &exact, ignore_case) {
            continue;
        }
        clear_node(doc, node, &exact, ignore_case)?;
        cleared += 1;
    }
    Ok(cleared)
}

fn collect_fields(doc: &mut PdfDocument) -> PdfResult<Vec<FieldNode>> {
    let catalog = doc.catalog()?;
    let Some(acro) = catalog.get("AcroForm").cloned() else {
        return Ok(Vec::new());
    };
    let acro_dict = match acro {
        PdfObject::Dictionary(dict) => dict,
        PdfObject::Reference(id) => match doc.get_object(id) {
            Ok(PdfObject::Dictionary(dict)) => dict,
            _ => return Ok(Vec::new()),
        },
        _ => return Ok(Vec::new()),
    };
    let Some(fields) = acro_dict.get("Fields").cloned() else {
        return Ok(Vec::new());
    };

    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    for id in array_ids(doc, &fields) {
        walk_field(doc, id, 0, &mut seen, &mut nodes);
    }
    Ok(nodes)
}

fn walk_field(
    doc: &mut PdfDocument,
    id: ObjectId,
    depth: usize,
    seen: &mut HashSet<ObjectId>,
    nodes: &mut Vec<FieldNode>,
) {
    if depth > MAX_FIELD_DEPTH || !seen.insert(id) {
        return;
    }
    let dict = match doc.get_object(id) {
        Ok(PdfObject::Dictionary(dict)) => dict,
        _ => return,
    };

    let mut widgets = Vec::new();
    let mut children = Vec::new();
    for kid in kid_ids(doc, &dict) {
        match doc.get_object(kid) {
            Ok(PdfObject::Dictionary(kid_dict)) if kid_dict.contains_key("T") => children.push(kid),
            Ok(PdfObject::Dictionary(_)) => widgets.push(kid),
            _ => {}
        }
    }
    nodes.push(FieldNode { id, widgets });
    for child in children {
        walk_field(doc, child, depth + 1, seen, nodes);
    }
}

fn node_repeats(
    doc: &mut PdfDocument,
    node: &FieldNode,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> bool {
    dict_repeats(doc, node.id, exact, ignore_case)
        || node
            .widgets
            .iter()
            .any(|widget| dict_repeats(doc, *widget, exact, ignore_case))
}

fn dict_repeats(
    doc: &mut PdfDocument,
    id: ObjectId,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> bool {
    let dict = match doc.get_object(id) {
        Ok(PdfObject::Dictionary(dict)) => dict,
        _ => return false,
    };
    if dict
        .get("V")
        .is_some_and(|value| value_repeats(doc, value, exact, ignore_case))
    {
        return true;
    }
    if dict
        .get("AP")
        .is_some_and(|value| appearance_repeats(doc, value, exact, ignore_case))
    {
        return true;
    }
    dict.get("AS")
        .is_some_and(|value| name_repeats(doc, value, exact, ignore_case))
}

fn clear_node(
    doc: &mut PdfDocument,
    node: &FieldNode,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> PdfResult<()> {
    let mut keep = vec![node.id];
    keep.extend(node.widgets.iter().copied());
    clear_dict(doc, node.id, &keep, exact, ignore_case)?;
    for widget in &node.widgets {
        clear_dict(doc, *widget, &keep, exact, ignore_case)?;
    }
    Ok(())
}

fn clear_dict(
    doc: &mut PdfDocument,
    id: ObjectId,
    keep: &[ObjectId],
    exact: &[&str],
    ignore_case: Option<&str>,
) -> PdfResult<()> {
    let mut dict = match doc.get_object(id) {
        Ok(PdfObject::Dictionary(dict)) => dict,
        _ => return Ok(()),
    };
    let clear_state = dict
        .get("AS")
        .is_some_and(|state| name_repeats(doc, state, exact, ignore_case));
    let old_value = dict.remove("V");
    let old_appearance = dict.remove("AP");
    let old_state = if clear_state { dict.remove("AS") } else { None };
    doc.set_object(id, PdfObject::Dictionary(dict));

    if let Some(value) = old_value {
        drop_value_object(doc, &value, keep);
    }
    if let Some(appearance) = old_appearance {
        let hit = collect_appearance(doc, &appearance);
        for target in hit.stream_ids.into_iter().chain(hit.dict_ids) {
            if !keep.contains(&target) {
                forget_object(doc, target);
            }
        }
    }
    if let Some(state) = old_state {
        drop_value_object(doc, &state, keep);
    }
    Ok(())
}

/// Drops an indirect value object. A signature dictionary stays in the file.
fn drop_value_object(doc: &mut PdfDocument, value: &PdfObject, keep: &[ObjectId]) {
    let Some(id) = value.as_reference() else {
        return;
    };
    if keep.contains(&id) {
        return;
    }
    let Ok(obj) = doc.get_object(id) else {
        forget_object(doc, id);
        return;
    };
    match obj {
        PdfObject::Dictionary(_) => {}
        PdfObject::Array(items) => {
            let members = items.clone();
            forget_object(doc, id);
            for member in members {
                drop_leaf_reference(doc, &member, keep);
            }
        }
        PdfObject::String(_)
        | PdfObject::Name(_)
        | PdfObject::Integer(_)
        | PdfObject::Real(_)
        | PdfObject::Boolean(_)
        | PdfObject::Stream(_) => forget_object(doc, id),
        _ => {}
    }
}

fn drop_leaf_reference(doc: &mut PdfDocument, value: &PdfObject, keep: &[ObjectId]) {
    let Some(id) = value.as_reference() else {
        return;
    };
    if keep.contains(&id) {
        return;
    }
    let Ok(obj) = doc.get_object(id) else {
        return;
    };
    if matches!(
        obj,
        PdfObject::String(_)
            | PdfObject::Name(_)
            | PdfObject::Integer(_)
            | PdfObject::Real(_)
            | PdfObject::Boolean(_)
            | PdfObject::Stream(_)
            | PdfObject::Array(_)
    ) {
        forget_object(doc, id);
    }
}

fn forget_object(doc: &mut PdfDocument, id: ObjectId) {
    doc.objects.remove(&id);
    doc.xref.entries.remove(&id);
}

fn value_repeats(
    doc: &mut PdfDocument,
    value: &PdfObject,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> bool {
    let Some(resolved) = resolve_value(doc, value, 0) else {
        return false;
    };
    match resolved {
        PdfObject::Stream(stream) => stream_repeats(doc, &stream, exact, ignore_case),
        PdfObject::Dictionary(_) => false,
        other => embedded::text_repeats(&value_text(doc, &other, 0), exact, ignore_case),
    }
}

fn name_repeats(
    doc: &mut PdfDocument,
    value: &PdfObject,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> bool {
    let Some(resolved) = resolve_value(doc, value, 0) else {
        return false;
    };
    let text = match resolved {
        PdfObject::Name(name) => name.0,
        PdfObject::String(text) => text.to_string_lossy(),
        _ => return false,
    };
    embedded::text_repeats(&text, exact, ignore_case)
}

fn value_text(doc: &mut PdfDocument, value: &PdfObject, depth: usize) -> String {
    if depth > MAX_VALUE_DEPTH {
        return String::new();
    }
    match value {
        PdfObject::String(text) => text.to_string_lossy(),
        PdfObject::Name(name) => name.0.clone(),
        PdfObject::Integer(number) => number.to_string(),
        PdfObject::Real(number) => format!("{number:.2}"),
        PdfObject::Boolean(true) => "Yes".to_string(),
        PdfObject::Boolean(false) => "Off".to_string(),
        PdfObject::Array(items) => items
            .iter()
            .map(|item| value_text(doc, item, depth + 1))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        PdfObject::Reference(id) => match doc.get_object(*id) {
            Ok(obj) => value_text(doc, &obj, depth + 1),
            Err(_) => String::new(),
        },
        PdfObject::Stream(stream) => decode_appearance(doc, stream)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn resolve_value(doc: &mut PdfDocument, value: &PdfObject, depth: usize) -> Option<PdfObject> {
    if depth > MAX_VALUE_DEPTH {
        return None;
    }
    match value {
        PdfObject::Reference(id) => {
            let obj = doc.get_object(*id).ok()?;
            resolve_value(doc, &obj, depth + 1)
        }
        other => Some(other.clone()),
    }
}

fn appearance_repeats(
    doc: &mut PdfDocument,
    value: &PdfObject,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> bool {
    collect_appearance(doc, value)
        .streams
        .iter()
        .any(|stream| stream_repeats(doc, stream, exact, ignore_case))
}

fn stream_repeats(
    doc: &PdfDocument,
    stream: &PdfStream,
    exact: &[&str],
    ignore_case: Option<&str>,
) -> bool {
    let Some(decoded) = decode_appearance(doc, stream) else {
        return false;
    };
    if embedded::payload_repeats(&decoded, exact, ignore_case) {
        return true;
    }
    appearance_text(&decoded)
        .is_some_and(|text| embedded::text_repeats(&text, exact, ignore_case))
}

fn decode_appearance(doc: &PdfDocument, stream: &PdfStream) -> Option<Vec<u8>> {
    let params = stream
        .dict
        .get("DecodeParms")
        .and_then(|item| item.as_dict());
    let filter = match stream.dict.get("Filter") {
        None => "",
        Some(PdfObject::Name(name)) => name.as_str(),
        _ => return None,
    };
    decode_stream(filter, params, &stream.content, &doc.limits).ok()
}

fn appearance_text(bytes: &[u8]) -> Option<String> {
    let mut tokenizer = ContentStreamTokenizer::new(bytes);
    let ops = tokenizer.tokenize_all().ok()?;
    let ast = build_ast_from_operations(ops);
    let paragraphs = LayoutReconstructor::new(&ast).reconstruct().ok()?;
    let mut text = String::new();
    for paragraph in paragraphs {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&paragraph.text());
    }
    Some(text)
}

fn collect_appearance(doc: &mut PdfDocument, value: &PdfObject) -> AppearanceHit {
    let mut hit = AppearanceHit {
        streams: Vec::new(),
        stream_ids: Vec::new(),
        dict_ids: Vec::new(),
    };
    let mut seen = HashSet::new();
    visit_appearance(doc, value, 0, &mut seen, &mut hit);
    hit
}

fn visit_appearance(
    doc: &mut PdfDocument,
    value: &PdfObject,
    depth: usize,
    seen: &mut HashSet<ObjectId>,
    hit: &mut AppearanceHit,
) {
    if depth > MAX_VALUE_DEPTH {
        return;
    }
    if let Some(id) = value.as_reference() {
        if !seen.insert(id) {
            return;
        }
        let Ok(obj) = doc.get_object(id) else {
            return;
        };
        match obj {
            PdfObject::Stream(stream) => {
                hit.stream_ids.push(id);
                hit.streams.push(stream);
            }
            PdfObject::Dictionary(dict) => {
                if appearance_dict(&dict) || state_map(&dict) {
                    hit.dict_ids.push(id);
                    walk_appearance_dict(doc, &dict, depth + 1, seen, hit);
                }
            }
            _ => {}
        }
        return;
    }
    match value {
        PdfObject::Stream(stream) => hit.streams.push(stream.clone()),
        PdfObject::Dictionary(dict) if appearance_dict(dict) || state_map(dict) => {
            walk_appearance_dict(doc, dict, depth + 1, seen, hit);
        }
        _ => {}
    }
}

fn walk_appearance_dict(
    doc: &mut PdfDocument,
    dict: &PdfDictionary,
    depth: usize,
    seen: &mut HashSet<ObjectId>,
    hit: &mut AppearanceHit,
) {
    if appearance_dict(dict) {
        for key in ["N", "R", "D"] {
            if let Some(child) = dict.get(key) {
                visit_appearance(doc, child, depth, seen, hit);
            }
        }
        return;
    }
    let children: Vec<PdfObject> = dict.iter().map(|(_, child)| child.clone()).collect();
    for child in children {
        visit_appearance(doc, &child, depth, seen, hit);
    }
}

fn appearance_dict(dict: &PdfDictionary) -> bool {
    dict.contains_key("N") || dict.contains_key("R") || dict.contains_key("D")
}

/// A state map is `/Yes 10 0 R /Off 11 0 R`. Resource dictionaries are not maps.
fn state_map(dict: &PdfDictionary) -> bool {
    if dict.contains_key("Resources")
        || dict.contains_key("Type")
        || dict.contains_key("Subtype")
        || dict.contains_key("Filter")
        || dict.contains_key("BBox")
        || dict.contains_key("Length")
    {
        return false;
    }
    dict.iter().all(|(_, value)| {
        matches!(
            value,
            PdfObject::Reference(_) | PdfObject::Stream(_) | PdfObject::Null
        )
    })
}

fn kid_ids(doc: &mut PdfDocument, dict: &PdfDictionary) -> Vec<ObjectId> {
    match dict.get("Kids") {
        Some(kids) => array_ids(doc, kids),
        None => Vec::new(),
    }
}

fn array_ids(doc: &mut PdfDocument, value: &PdfObject) -> Vec<ObjectId> {
    let items = match value {
        PdfObject::Array(items) => items.clone(),
        PdfObject::Reference(id) => match doc.get_object(*id) {
            Ok(PdfObject::Array(items)) => items,
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    items.iter().filter_map(|item| item.as_reference()).collect()
}
