//! Structural PDF/A-1b and PDF/A-2b conversion.
//!
//! `convert_to_pdfa` mutates a clone and replaces the caller's document only
//! when that clone has no remaining issues. A live session therefore keeps
//! its previous objects when archive conversion is refused.

use std::collections::{HashMap, HashSet};

use crate::cos::{
    encode_flate, Compression, ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject,
    PdfStream, PdfString, XRefEntry,
};
use crate::error::{PdfError, PdfResult};
use crate::fonts::WinAnsiEncoding;

const ENCRYPTED: &str = "An encrypted document cannot be archived.";
const COMPOSITE: &str = "A composite font cannot be embedded for archive.";
const ENCODING: &str = "A custom font encoding cannot be archived.";
const APPEARANCE: &str = "An annotation has no appearance.";
const NOT_ARCHIVE: &str = "The document is not an archive after conversion.";
const MISSING_ID: &str = "A trailer identifier is missing.";
const MISSING_METADATA: &str = "Archive metadata is missing.";
const MISSING_INTENT: &str = "An output intent is missing.";
const TRANSPARENCY: &str = "Transparency is not allowed in this archive part.";
const EMBEDDED_FILE: &str = "An embedded file is not allowed.";
const ACTIVE: &str = "An active action is not allowed.";
const NOT_EMBEDDED: &str = "A font is not embedded.";
const ANNOT_FORBIDDEN: &str = "An annotation is not allowed in this archive.";

const DROPPED_KEYS: &[&str] = &[
    "JavaScript",
    "JS",
    "Launch",
    "SubmitForm",
    "RichMedia",
    "Movie",
    "Sound",
    "Rendition",
    "OpenAction",
    "AA",
    "OCProperties",
    "EmbeddedFiles",
    "EF",
    "NeedAppearances",
];

const HARD_FAILURES: &[&str] = &[ENCRYPTED, COMPOSITE, ENCODING, APPEARANCE];

/// PDF/A part the converter knows how to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfALevel {
    /// ISO 19005-1 level B. Transparency is removed and the header is 1.4.
    A1b,
    /// ISO 19005-2 level B. Transparency is kept and the header is 1.7.
    A2b,
}

impl PdfALevel {
    /// Parses `1b` or `2b`. Any other token is rejected without being echoed.
    pub fn parse(part: &str) -> Option<Self> {
        match part {
            "1b" => Some(Self::A1b),
            "2b" => Some(Self::A2b),
            _ => None,
        }
    }

    /// `pdfaid:part` value, 1 or 2.
    pub fn part_number(self) -> u8 {
        match self {
            Self::A1b => 1,
            Self::A2b => 2,
        }
    }

    /// Header version written when this part is saved.
    pub fn pdf_version(self) -> &'static str {
        match self {
            Self::A1b => "1.4",
            Self::A2b => "1.7",
        }
    }

    /// Short label for an operator message.
    pub fn label(self) -> &'static str {
        match self {
            Self::A1b => "PDF/A-1b",
            Self::A2b => "PDF/A-2b",
        }
    }

    fn forbids_transparency(self) -> bool {
        matches!(self, Self::A1b)
    }
}

/// Rewrites `doc` so a later save carries `level`.
///
/// The caller's document is replaced only after the rewritten copy validates.
/// Refusal, including a composite font or a missing appearance, leaves the
/// original object graph unchanged.
pub fn convert_to_pdfa(doc: &mut PdfDocument, level: PdfALevel) -> PdfResult<()> {
    if doc.xref.trailer.contains_key("Encrypt") {
        return Err(operation(ENCRYPTED));
    }
    let mut working = doc.clone();
    materialize(&mut working)?;
    // `/S /Transparency` is deleted by the scrub, so the object ids are recorded first.
    let (transparency, actions) = record_special(&working);
    scrub_objects(&mut working, level, &transparency)?;
    null_rejected(&mut working, &actions)?;
    ensure_appearances(&mut working)?;
    embed_fonts(&mut working)?;
    install_archive_info(&mut working, level)?;
    normalize_lengths(&mut working)?;
    let issues = validate_pdfa(&mut working, level)?;
    if !issues.is_empty() {
        return Err(rejection_from(&issues));
    }
    *doc = working;
    Ok(())
}

/// Reports structural archive issues.
///
/// Parsing caches objects on `doc`. The function does not strip, embed, or
/// rewrite, and an empty list is not a third-party conformance certificate.
pub fn validate_pdfa(doc: &mut PdfDocument, level: PdfALevel) -> PdfResult<Vec<String>> {
    let mut issues = Vec::new();
    if doc.xref.trailer.contains_key("Encrypt") {
        push_issue(&mut issues, ENCRYPTED);
    }
    if !trailer_id_present(doc) {
        push_issue(&mut issues, MISSING_ID);
    }
    materialize(doc)?;
    let mut groups = Vec::new();
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let max_depth = doc.limits.max_recursion_depth;
    for id in ids {
        let object = doc.get_object(id)?;
        if let Some(dict) = object.as_dict().cloned() {
            if is_font_dict(&dict) {
                check_font(&dict, doc, &mut issues)?;
            }
            if is_annotation(&dict) {
                check_annotation(&dict, &mut issues);
            }
        }
        let object = doc.get_object(id)?;
        scan_tree(&object, 0, max_depth, id, level, &mut issues, &mut groups)?;
    }
    for group_id in groups {
        let object = doc.get_object(group_id)?;
        let transparent = object
            .as_dict()
            .and_then(|dict| dict.get("S"))
            .and_then(|value| value.as_name())
            == Some("Transparency");
        if transparent {
            push_issue(&mut issues, TRANSPARENCY);
        }
    }
    match doc.catalog_id() {
        Some(catalog_id) => match doc.get_object(catalog_id)? {
            PdfObject::Dictionary(dict) => {
                if !metadata_present(doc, &dict, level.part_number())? {
                    push_issue(&mut issues, MISSING_METADATA);
                }
                if !output_intent_present(doc, &dict)? {
                    push_issue(&mut issues, MISSING_INTENT);
                }
            }
            _ => {
                push_issue(&mut issues, MISSING_METADATA);
                push_issue(&mut issues, MISSING_INTENT);
            }
        },
        None => {
            push_issue(&mut issues, MISSING_METADATA);
            push_issue(&mut issues, MISSING_INTENT);
        }
    }
    Ok(issues)
}

fn operation(message: &str) -> PdfError {
    PdfError::OperationError(message.to_string())
}

fn rejection_from(issues: &[String]) -> PdfError {
    for sentence in HARD_FAILURES {
        if issues.iter().any(|issue| issue == sentence) {
            return operation(sentence);
        }
    }
    operation(NOT_ARCHIVE)
}

fn push_issue(issues: &mut Vec<String>, sentence: &str) {
    if !issues.iter().any(|issue| issue == sentence) {
        issues.push(sentence.to_string());
    }
}

fn materialize(doc: &mut PdfDocument) -> PdfResult<()> {
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
        let _ = doc.get_object(id)?;
    }
    Ok(())
}

fn record_special(doc: &PdfDocument) -> (HashSet<ObjectId>, HashSet<ObjectId>) {
    let mut transparency = HashSet::new();
    let mut actions = HashSet::new();
    for (id, object) in &doc.objects {
        let Some(dict) = object.as_dict() else {
            continue;
        };
        let subtype = dict
            .get("S")
            .and_then(|value| value.as_name())
            .map(str::to_string);
        if subtype.as_deref() == Some("Transparency") {
            transparency.insert(*id);
        }
        let dangerous = matches!(
            subtype.as_deref(),
            Some("JavaScript" | "Launch" | "SubmitForm")
        );
        let javascript =
            dict.get("Subtype").and_then(|value| value.as_name()) == Some("JavaScript");
        if (dangerous || javascript) && !is_structural(dict) {
            actions.insert(*id);
        }
    }
    (transparency, actions)
}

fn scrub_objects(
    doc: &mut PdfDocument,
    level: PdfALevel,
    transparency: &HashSet<ObjectId>,
) -> PdfResult<()> {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let max_depth = doc.limits.max_recursion_depth;
    for id in ids {
        let mut object = doc.get_object(id)?;
        scrub_value(&mut object, level, transparency, 0, max_depth, id)?;
        doc.set_object(id, object);
    }
    Ok(())
}

fn scrub_value(
    value: &mut PdfObject,
    level: PdfALevel,
    transparency: &HashSet<ObjectId>,
    depth: usize,
    max_depth: usize,
    object_id: ObjectId,
) -> PdfResult<()> {
    if depth > max_depth {
        return Err(PdfError::RecursionLimitExceeded {
            id: object_id.number,
            gen: object_id.generation,
            max_depth,
        });
    }
    match value {
        PdfObject::Dictionary(dict) => {
            scrub_dict(dict, level, transparency, depth, max_depth, object_id)
        }
        PdfObject::Stream(stream) => scrub_dict(
            &mut stream.dict,
            level,
            transparency,
            depth,
            max_depth,
            object_id,
        ),
        PdfObject::Array(items) => {
            for item in items.iter_mut() {
                scrub_value(item, level, transparency, depth + 1, max_depth, object_id)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn scrub_dict(
    dict: &mut PdfDictionary,
    level: PdfALevel,
    transparency: &HashSet<ObjectId>,
    depth: usize,
    max_depth: usize,
    object_id: ObjectId,
) -> PdfResult<()> {
    if level.forbids_transparency() {
        force_opaque(dict);
        // Nested `/S` would disappear if the group were walked before this test.
        if group_should_drop(dict, transparency) {
            dict.remove("Group");
        }
        let group_s = dict.get("S").and_then(|value| value.as_name()) == Some("Transparency");
        if group_s {
            dict.remove("S");
        }
    }
    for key in DROPPED_KEYS {
        dict.remove(*key);
    }
    let dangerous = matches!(
        dict.get("S").and_then(|value| value.as_name()),
        Some("JavaScript" | "Launch" | "SubmitForm")
    );
    if dangerous {
        dict.remove("S");
    }
    if is_annotation(dict) {
        let flags = dict
            .get("F")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        dict.insert("F", flags.round() as i64 | 4);
    }
    let keys = owned_keys(dict);
    for key in keys {
        if let Some(child) = dict.get_mut(&key) {
            scrub_value(child, level, transparency, depth + 1, max_depth, object_id)?;
        }
    }
    Ok(())
}

fn force_opaque(dict: &mut PdfDictionary) {
    for key in ["ca", "CA"] {
        let value = dict.get(key).and_then(|item| item.as_f64());
        if let Some(value) = value {
            if (value - 1.0).abs() > 0.001 {
                dict.insert(key, 1i64);
            }
        }
    }
    let has_blend = dict.contains_key("BM");
    let normal = dict.get("BM").and_then(|item| item.as_name()) == Some("Normal");
    if has_blend && !normal {
        dict.insert("BM", PdfName::new("Normal"));
    }
    dict.remove("SMask");
}

fn group_should_drop(dict: &PdfDictionary, transparency: &HashSet<ObjectId>) -> bool {
    match dict.get("Group") {
        Some(PdfObject::Reference(id)) => transparency.contains(id),
        Some(PdfObject::Dictionary(inner)) => {
            inner.get("S").and_then(|value| value.as_name()) == Some("Transparency")
        }
        _ => false,
    }
}

fn owned_keys(dict: &PdfDictionary) -> Vec<String> {
    dict.iter()
        .map(|(name, _)| name.as_str().to_string())
        .collect()
}

fn null_rejected(doc: &mut PdfDocument, actions: &HashSet<ObjectId>) -> PdfResult<()> {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let mut doomed = actions.clone();
    for id in &ids {
        let object = doc.get_object(*id)?;
        let Some(dict) = object.as_dict() else {
            continue;
        };
        let type_name = dict
            .get("Type")
            .and_then(|value| value.as_name())
            .map(str::to_string);
        let subtype = dict
            .get("Subtype")
            .and_then(|value| value.as_name())
            .map(str::to_string);
        if type_name.as_deref() == Some("EmbeddedFile")
            || subtype.as_deref() == Some("EmbeddedFile")
        {
            doomed.insert(*id);
        }
        if is_disallowed_annot(subtype.as_deref()) {
            doomed.insert(*id);
        }
    }
    for id in doomed {
        doc.set_object(id, PdfObject::Null);
    }
    filter_annots(doc)
}

fn filter_annots(doc: &mut PdfDocument) -> PdfResult<()> {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let mut object = doc.get_object(id)?;
        let Some(dict) = object.as_dict() else {
            continue;
        };
        if !dict.contains_key("Annots") {
            continue;
        }
        let Some(existing) = dict
            .get("Annots")
            .and_then(|value| value.as_array())
            .map(|items| items.to_vec())
        else {
            continue;
        };
        let mut kept = Vec::new();
        for item in existing {
            if annot_item_rejected(&item, doc)? {
                continue;
            }
            kept.push(item);
        }
        if let Some(dict) = object.as_dict_mut() {
            dict.insert("Annots", PdfObject::Array(kept));
        }
        doc.set_object(id, object);
    }
    Ok(())
}

fn annot_item_rejected(item: &PdfObject, doc: &mut PdfDocument) -> PdfResult<bool> {
    if let Some(id) = item.as_reference() {
        let target = doc.get_object(id)?;
        if target.is_null() {
            return Ok(true);
        }
        if let Some(dict) = target.as_dict() {
            let subtype = dict
                .get("Subtype")
                .and_then(|value| value.as_name())
                .map(str::to_string);
            return Ok(is_disallowed_annot(subtype.as_deref()));
        }
        return Ok(false);
    }
    if let Some(dict) = item.as_dict() {
        let subtype = dict
            .get("Subtype")
            .and_then(|value| value.as_name())
            .map(str::to_string);
        return Ok(is_disallowed_annot(subtype.as_deref()));
    }
    Ok(false)
}

fn ensure_appearances(doc: &mut PdfDocument) -> PdfResult<()> {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let object = doc.get_object(id)?;
        let Some(dict) = object.as_dict().cloned() else {
            continue;
        };
        if !is_annotation(&dict) {
            continue;
        }
        let subtype = dict
            .get("Subtype")
            .and_then(|value| value.as_name())
            .map(str::to_string);
        if subtype.as_deref() == Some("Popup") || is_disallowed_annot(subtype.as_deref()) {
            continue;
        }
        if has_appearance(&dict) {
            continue;
        }
        if subtype.as_deref() == Some("Link") {
            synthesize_link(doc, id, &dict)?;
        } else {
            return Err(operation(APPEARANCE));
        }
    }
    Ok(())
}

fn has_appearance(dict: &PdfDictionary) -> bool {
    match dict.get("AP") {
        None | Some(PdfObject::Null) => false,
        Some(_) => true,
    }
}

fn synthesize_link(doc: &mut PdfDocument, id: ObjectId, dict: &PdfDictionary) -> PdfResult<()> {
    let Some(rect) = rect_edges(dict, doc)? else {
        return Err(operation(APPEARANCE));
    };
    let width = span(rect[0], rect[2]);
    let height = span(rect[1], rect[3]);
    let border = border_width(dict, doc)?;
    let color = stroke_color(dict);
    let content = match border {
        BorderWidth::Width(stroke) if stroke >= 0.5 => format!(
            "{} {} {} RG\n{} w\n0 0 {} {} re\nS\n",
            pdf_number(color[0]),
            pdf_number(color[1]),
            pdf_number(color[2]),
            pdf_number(stroke),
            pdf_number(width),
            pdf_number(height),
        ),
        _ => "q\nQ\n".to_string(),
    };
    let bytes = content.into_bytes();
    let mut form = PdfDictionary::new();
    form.insert("Type", PdfName::new("XObject"));
    form.insert("Subtype", PdfName::new("Form"));
    form.insert("FormType", 1i64);
    form.insert(
        "BBox",
        PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            number_object(width),
            number_object(height),
        ]),
    );
    form.insert("Length", bytes.len() as i64);
    let form_id = doc.alloc_object_id();
    doc.set_object(form_id, PdfObject::Stream(PdfStream::new(form, bytes)));
    let mut annot = dict.clone();
    let mut appearance = PdfDictionary::new();
    appearance.insert("N", form_id);
    annot.insert("AP", PdfObject::Dictionary(appearance));
    doc.set_object(id, PdfObject::Dictionary(annot));
    Ok(())
}

enum BorderWidth {
    None,
    Width(f64),
}

fn border_width(dict: &PdfDictionary, doc: &mut PdfDocument) -> PdfResult<BorderWidth> {
    if dict.contains_key("BS") {
        let Some(raw) = dict.get("BS").cloned() else {
            return Ok(BorderWidth::Width(0.0));
        };
        let Some(bs) = resolve_dict(&raw, doc)? else {
            return Ok(BorderWidth::Width(0.0));
        };
        if let Some(width) = bs.get("W").and_then(|value| value.as_f64()) {
            return Ok(BorderWidth::Width(width));
        }
        return Ok(BorderWidth::Width(0.0));
    }
    if dict.contains_key("Border") {
        let Some(raw) = dict.get("Border").cloned() else {
            return Ok(BorderWidth::Width(0.0));
        };
        let array = if let Some(id) = raw.as_reference() {
            doc.get_object(id)?
        } else {
            raw
        };
        if let Some(items) = array.as_array() {
            if let Some(width) = items.get(2).and_then(|value| value.as_f64()) {
                return Ok(BorderWidth::Width(width));
            }
        }
        return Ok(BorderWidth::Width(0.0));
    }
    Ok(BorderWidth::None)
}

fn rect_edges(dict: &PdfDictionary, doc: &mut PdfDocument) -> PdfResult<Option<[f64; 4]>> {
    let Some(raw) = dict.get("Rect").cloned() else {
        return Ok(None);
    };
    let value = if let Some(id) = raw.as_reference() {
        doc.get_object(id)?
    } else {
        raw
    };
    let Some(array) = value.as_array() else {
        return Ok(None);
    };
    if array.len() < 4 {
        return Ok(None);
    }
    let mut edges = [0.0; 4];
    for (index, slot) in edges.iter_mut().enumerate() {
        let Some(number) = array[index].as_f64() else {
            return Ok(None);
        };
        *slot = number;
    }
    Ok(Some(edges))
}

fn span(start: f64, end: f64) -> f64 {
    let size = (end - start).abs();
    if size < 1.0 {
        1.0
    } else {
        size
    }
}

fn stroke_color(dict: &PdfDictionary) -> [f64; 3] {
    let Some(array) = dict.get("C").and_then(|value| value.as_array()) else {
        return [0.0, 0.0, 0.0];
    };
    if array.len() < 3 {
        return [0.0, 0.0, 0.0];
    }
    match (array[0].as_f64(), array[1].as_f64(), array[2].as_f64()) {
        (Some(red), Some(green), Some(blue)) => [red, green, blue],
        _ => [0.0, 0.0, 0.0],
    }
}

fn pdf_number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut text = format!("{value:.5}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" || text.is_empty() {
        "0".to_string()
    } else {
        text
    }
}

fn number_object(value: f64) -> PdfObject {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < (i64::MAX as f64) {
        PdfObject::Integer(value as i64)
    } else if value.is_finite() {
        PdfObject::Real(value)
    } else {
        PdfObject::Integer(0)
    }
}

fn embed_fonts(doc: &mut PdfDocument) -> PdfResult<()> {
    let ids = font_ids(doc);
    let mut faces: HashMap<[i32; 224], SharedFace> = HashMap::new();
    let mut next_tag = 0u32;
    let mut unicode_id = None;
    for id in ids {
        let object = doc.get_object(id)?;
        let Some(dict) = object.as_dict().cloned() else {
            continue;
        };
        if is_composite_font(&dict) {
            if !composite_has_program(&dict, doc)? {
                return Err(operation(COMPOSITE));
            }
            continue;
        }
        if !encoding_allowed(&dict, doc)? {
            return Err(operation(ENCODING));
        }
        if simple_embedded(&dict, doc)? {
            continue;
        }
        let widths = archive_widths(&dict, doc)?;
        let unicode = if let Some(existing) = dict
            .get("ToUnicode")
            .filter(|value| !value.is_null())
            .cloned()
        {
            existing
        } else {
            PdfObject::Reference(shared_unicode(doc, &mut unicode_id)?)
        };
        let found = faces.get(&widths).cloned();
        let shared = if let Some(existing) = found {
            existing
        } else {
            let created = store_face(doc, &widths, next_tag)?;
            next_tag += 1;
            faces.insert(widths, created.clone());
            created
        };
        let font = rewritten_font(&shared.base_font, &widths, shared.descriptor_id, unicode);
        doc.set_object(id, PdfObject::Dictionary(font));
    }
    Ok(())
}

#[derive(Clone)]
struct SharedFace {
    base_font: String,
    descriptor_id: ObjectId,
}

fn font_ids(doc: &PdfDocument) -> Vec<ObjectId> {
    doc.objects
        .iter()
        .filter_map(|(id, object)| {
            let dict = object.as_dict()?;
            if is_font_dict(dict) {
                Some(*id)
            } else {
                None
            }
        })
        .collect()
}

fn store_face(doc: &mut PdfDocument, widths: &[i32; 224], tag_index: u32) -> PdfResult<SharedFace> {
    let face = super::face::build_face(widths, tag_index);
    let compressed = encode_flate(&face.bytes, Compression::best())?;
    let mut file_dict = PdfDictionary::new();
    file_dict.insert("Length", compressed.len() as i64);
    file_dict.insert("Length1", face.bytes.len() as i64);
    file_dict.insert("Filter", PdfName::new("FlateDecode"));
    let file_id = doc.alloc_object_id();
    doc.set_object(
        file_id,
        PdfObject::Stream(PdfStream::new(file_dict, compressed)),
    );

    let mut descriptor = PdfDictionary::new();
    descriptor.insert("Type", PdfName::new("FontDescriptor"));
    descriptor.insert("FontName", PdfName::new(face.base_font.clone()));
    descriptor.insert("Flags", if face.fixed_pitch { 33i64 } else { 32i64 });
    let bbox: PdfArray = face
        .bbox
        .iter()
        .map(|value| PdfObject::Integer(i64::from(*value)))
        .collect();
    descriptor.insert("FontBBox", PdfObject::Array(bbox));
    descriptor.insert("ItalicAngle", 0i64);
    descriptor.insert("Ascent", i64::from(face.ascent));
    descriptor.insert("Descent", i64::from(face.descent));
    descriptor.insert("CapHeight", i64::from(face.cap_height));
    descriptor.insert("StemV", i64::from(face.stem_v.max(1)));
    descriptor.insert("FontFile2", file_id);
    let descriptor_id = doc.alloc_object_id();
    doc.set_object(descriptor_id, PdfObject::Dictionary(descriptor));
    Ok(SharedFace {
        base_font: face.base_font,
        descriptor_id,
    })
}

fn rewritten_font(
    base_font: &str,
    widths: &[i32; 224],
    descriptor_id: ObjectId,
    unicode: PdfObject,
) -> PdfDictionary {
    let mut font = PdfDictionary::new();
    font.insert("Type", PdfName::new("Font"));
    font.insert("Subtype", PdfName::new("TrueType"));
    font.insert("BaseFont", PdfName::new(base_font));
    font.insert("Encoding", PdfName::new("WinAnsiEncoding"));
    font.insert("FirstChar", 32i64);
    font.insert("LastChar", 255i64);
    let mut array = PdfArray::new();
    for width in widths {
        array.push(PdfObject::Integer(i64::from(*width)));
    }
    font.insert("Widths", PdfObject::Array(array));
    font.insert("FontDescriptor", descriptor_id);
    font.insert("ToUnicode", unicode);
    font
}

fn shared_unicode(doc: &mut PdfDocument, slot: &mut Option<ObjectId>) -> PdfResult<ObjectId> {
    if let Some(id) = *slot {
        return Ok(id);
    }
    let bytes = winansi_tounicode();
    let mut dict = PdfDictionary::new();
    dict.insert("Length", bytes.len() as i64);
    let id = doc.alloc_object_id();
    doc.set_object(id, PdfObject::Stream(PdfStream::new(dict, bytes)));
    *slot = Some(id);
    Ok(id)
}

fn winansi_tounicode() -> Vec<u8> {
    let codes: Vec<u8> = (32u8..=255).collect();
    let mut body = String::new();
    for chunk in codes.chunks(100) {
        body.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for code in chunk {
            let unicode = WinAnsiEncoding::code_to_unicode(*code) as u32;
            body.push_str(&format!("<{code:02X}> <{unicode:04X}>\n"));
        }
        body.push_str("endbfchar\n");
    }
    format!(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n\
         <00> <FF>\n\
         endcodespacerange\n\
         {body}\
         endcmap\n\
         CMapName currentdict /CMap defineresource pop\n\
         end\n\
         end\n"
    )
    .into_bytes()
}

fn archive_widths(dict: &PdfDictionary, doc: &mut PdfDocument) -> PdfResult<[i32; 224]> {
    let base = dict
        .get("BaseFont")
        .and_then(|value| value.as_name())
        .unwrap_or("");
    let mut widths = default_widths(base);
    let Some(raw) = dict.get("Widths").cloned() else {
        return Ok(widths);
    };
    let resolved = if let Some(id) = raw.as_reference() {
        doc.get_object(id)?
    } else {
        raw
    };
    let Some(array) = resolved.as_array() else {
        return Ok(widths);
    };
    let first = dict
        .get("FirstChar")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let last = dict.get("LastChar").and_then(|value| value.as_i64());
    for (index, item) in array.iter().enumerate() {
        let Some(value) = item.as_f64() else {
            continue;
        };
        let code = first + index as i64;
        if !(32..=255).contains(&code) {
            continue;
        }
        if last.is_some_and(|end| code > end) {
            break;
        }
        widths[(code - 32) as usize] = round_width(value);
    }
    Ok(widths)
}

fn default_widths(base_font: &str) -> [i32; 224] {
    let mut widths = [0i32; 224];
    if base_font.contains("Courier") {
        widths.fill(600);
        return widths;
    }
    for code in 32u8..=255 {
        let glyph = WinAnsiEncoding::code_to_unicode(code);
        let width = match glyph {
            ' ' | 'i' | 'l' | 'I' | 'j' | '.' | ',' | '\u{2019}' => 278,
            'm' | 'w' | 'M' | 'W' => 833,
            _ => 556,
        };
        widths[(code - 32) as usize] = width;
    }
    widths
}

fn round_width(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let rounded = value.round();
    if rounded <= 0.0 {
        0
    } else if rounded >= 4000.0 {
        4000
    } else {
        rounded as i32
    }
}

fn is_composite_font(dict: &PdfDictionary) -> bool {
    matches!(
        dict.get("Subtype").and_then(|value| value.as_name()),
        Some("Type0" | "CIDFontType0" | "CIDFontType2")
    )
}

fn composite_has_program(dict: &PdfDictionary, doc: &mut PdfDocument) -> PdfResult<bool> {
    let subtype = dict
        .get("Subtype")
        .and_then(|value| value.as_name())
        .unwrap_or("");
    if subtype == "CIDFontType0" || subtype == "CIDFontType2" {
        return descriptor_has_program(dict, doc);
    }
    let Some(kids) = dict
        .get("DescendantFonts")
        .and_then(|value| value.as_array())
        .map(|items| items.to_vec())
    else {
        return Ok(false);
    };
    if kids.is_empty() {
        return Ok(false);
    }
    for kid in &kids {
        let Some(kid_dict) = resolve_dict(kid, doc)? else {
            return Ok(false);
        };
        if !descriptor_has_program(&kid_dict, doc)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn descriptor_has_program(dict: &PdfDictionary, doc: &mut PdfDocument) -> PdfResult<bool> {
    let Some(raw) = dict.get("FontDescriptor").cloned() else {
        return Ok(false);
    };
    let Some(descriptor) = resolve_dict(&raw, doc)? else {
        return Ok(false);
    };
    Ok(has_program_key(&descriptor))
}

fn has_program_key(dict: &PdfDictionary) -> bool {
    ["FontFile", "FontFile2", "FontFile3"]
        .iter()
        .any(|key| match dict.get(*key) {
            Some(PdfObject::Null) | None => false,
            Some(_) => true,
        })
}

fn simple_embedded(dict: &PdfDictionary, doc: &mut PdfDocument) -> PdfResult<bool> {
    let subtype = dict
        .get("Subtype")
        .and_then(|value| value.as_name())
        .unwrap_or("");
    let char_procs = dict.get("CharProcs").is_some_and(|value| !value.is_null());
    if subtype == "Type3" && char_procs {
        return Ok(true);
    }
    descriptor_has_program(dict, doc)
}

fn encoding_allowed(dict: &PdfDictionary, doc: &mut PdfDocument) -> PdfResult<bool> {
    let Some(encoding) = dict.get("Encoding").cloned() else {
        return Ok(true);
    };
    match encoding {
        PdfObject::Name(name) => Ok(is_standard_encoding(name.as_str())),
        PdfObject::Reference(id) => match doc.get_object(id)? {
            PdfObject::Name(name) => Ok(is_standard_encoding(name.as_str())),
            _ => Ok(false),
        },
        _ => Ok(false),
    }
}

fn is_standard_encoding(name: &str) -> bool {
    name == "WinAnsiEncoding" || name == "StandardEncoding"
}

fn resolve_dict(object: &PdfObject, doc: &mut PdfDocument) -> PdfResult<Option<PdfDictionary>> {
    match object {
        PdfObject::Dictionary(dict) => Ok(Some(dict.clone())),
        PdfObject::Stream(stream) => Ok(Some(stream.dict.clone())),
        PdfObject::Reference(id) => match doc.get_object(*id)? {
            PdfObject::Dictionary(dict) => Ok(Some(dict)),
            PdfObject::Stream(stream) => Ok(Some(stream.dict)),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn install_archive_info(doc: &mut PdfDocument, level: PdfALevel) -> PdfResult<()> {
    let catalog_id = doc.catalog_id().ok_or_else(|| operation(NOT_ARCHIVE))?;
    let mut catalog = match doc.get_object(catalog_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => return Err(operation(NOT_ARCHIVE)),
    };
    let packet = xmp_packet(level.part_number());
    let mut meta_dict = PdfDictionary::new();
    meta_dict.insert("Type", PdfName::new("Metadata"));
    meta_dict.insert("Subtype", PdfName::new("XML"));
    meta_dict.insert("Length", packet.len() as i64);
    let meta_id = doc.alloc_object_id();
    doc.set_object(
        meta_id,
        PdfObject::Stream(PdfStream::new(meta_dict, packet)),
    );

    let profile = super::icc::srgb_profile();
    let mut profile_dict = PdfDictionary::new();
    profile_dict.insert("N", 3i64);
    profile_dict.insert("Length", profile.len() as i64);
    let profile_id = doc.alloc_object_id();
    doc.set_object(
        profile_id,
        PdfObject::Stream(PdfStream::new(profile_dict, profile)),
    );

    let mut intent = PdfDictionary::new();
    intent.insert("Type", PdfName::new("OutputIntent"));
    intent.insert("S", PdfName::new("GTS_PDFA1"));
    intent.insert(
        "OutputConditionIdentifier",
        PdfString::literal(b"sRGB".to_vec()),
    );
    intent.insert(
        "RegistryName",
        PdfString::literal(b"http://www.color.org".to_vec()),
    );
    intent.insert("Info", PdfString::literal(b"sRGB".to_vec()));
    intent.insert("DestOutputProfile", profile_id);
    catalog.insert("Metadata", meta_id);
    catalog.insert("OutputIntents", vec![PdfObject::Dictionary(intent)]);
    doc.set_object(catalog_id, PdfObject::Dictionary(catalog));
    doc.xref
        .trailer
        .insert("ID", trailer_identifier(doc, level.part_number()));
    doc.write_version = level.pdf_version().to_string();
    Ok(())
}

fn xmp_packet(part: u8) -> Vec<u8> {
    format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n\
         <pdfaid:part>{part}</pdfaid:part>\n\
         <pdfaid:conformance>B</pdfaid:conformance>\n\
         </rdf:Description>\n\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n"
    )
    .into_bytes()
}

fn trailer_identifier(doc: &PdfDocument, part: u8) -> PdfObject {
    let mut seed = b"PDFEngine-pdfa".to_vec();
    seed.extend_from_slice(&(doc.raw_data().len() as u64).to_be_bytes());
    seed.push(part);
    let digest = crate::crypto::sha256::sha256(&seed);
    let text = PdfString::hex(digest[..16].to_vec());
    PdfObject::Array(vec![
        PdfObject::String(text.clone()),
        PdfObject::String(text),
    ])
}

fn normalize_lengths(doc: &mut PdfDocument) -> PdfResult<()> {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let object = doc.get_object(id)?;
        let PdfObject::Stream(mut stream) = object else {
            continue;
        };
        stream.dict.insert("Length", stream.content.len() as i64);
        doc.set_object(id, PdfObject::Stream(stream));
    }
    Ok(())
}

fn scan_tree(
    object: &PdfObject,
    depth: usize,
    max_depth: usize,
    object_id: ObjectId,
    level: PdfALevel,
    issues: &mut Vec<String>,
    groups: &mut Vec<ObjectId>,
) -> PdfResult<()> {
    if depth > max_depth {
        return Err(PdfError::RecursionLimitExceeded {
            id: object_id.number,
            gen: object_id.generation,
            max_depth,
        });
    }
    match object {
        PdfObject::Dictionary(dict) => {
            note_dict(dict, level, issues, groups);
            for (_, value) in dict.iter() {
                scan_tree(
                    value,
                    depth + 1,
                    max_depth,
                    object_id,
                    level,
                    issues,
                    groups,
                )?;
            }
        }
        PdfObject::Stream(stream) => {
            note_dict(&stream.dict, level, issues, groups);
            for (_, value) in stream.dict.iter() {
                scan_tree(
                    value,
                    depth + 1,
                    max_depth,
                    object_id,
                    level,
                    issues,
                    groups,
                )?;
            }
        }
        PdfObject::Array(items) => {
            for item in items {
                scan_tree(item, depth + 1, max_depth, object_id, level, issues, groups)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn note_dict(
    dict: &PdfDictionary,
    level: PdfALevel,
    issues: &mut Vec<String>,
    groups: &mut Vec<ObjectId>,
) {
    if level.forbids_transparency() {
        for key in ["ca", "CA"] {
            let value = dict.get(key).and_then(|item| item.as_f64());
            if let Some(value) = value {
                if (value - 1.0).abs() > 0.001 {
                    push_issue(issues, TRANSPARENCY);
                }
            }
        }
        let has_blend = dict.contains_key("BM");
        let normal = dict.get("BM").and_then(|item| item.as_name()) == Some("Normal");
        if has_blend && !normal {
            push_issue(issues, TRANSPARENCY);
        }
        if dict.contains_key("SMask") {
            push_issue(issues, TRANSPARENCY);
        }
        if dict.get("S").and_then(|item| item.as_name()) == Some("Transparency") {
            push_issue(issues, TRANSPARENCY);
        }
        if let Some(id) = dict.get("Group").and_then(|item| item.as_reference()) {
            groups.push(id);
        }
    }
    if dict.contains_key("EmbeddedFiles") || dict.contains_key("EF") {
        push_issue(issues, EMBEDDED_FILE);
    }
    let type_name = dict
        .get("Type")
        .and_then(|item| item.as_name())
        .map(str::to_string);
    let subtype = dict
        .get("Subtype")
        .and_then(|item| item.as_name())
        .map(str::to_string);
    if type_name.as_deref() == Some("EmbeddedFile") || subtype.as_deref() == Some("EmbeddedFile") {
        push_issue(issues, EMBEDDED_FILE);
    }
    for key in [
        "JavaScript",
        "JS",
        "Launch",
        "SubmitForm",
        "OpenAction",
        "AA",
    ] {
        if dict.contains_key(key) {
            push_issue(issues, ACTIVE);
        }
    }
    if matches!(
        dict.get("S").and_then(|item| item.as_name()),
        Some("JavaScript" | "Launch" | "SubmitForm")
    ) {
        push_issue(issues, ACTIVE);
    }
    if subtype.as_deref() == Some("JavaScript") {
        push_issue(issues, ACTIVE);
    }
}

fn check_font(
    dict: &PdfDictionary,
    doc: &mut PdfDocument,
    issues: &mut Vec<String>,
) -> PdfResult<()> {
    if is_composite_font(dict) {
        if !composite_has_program(dict, doc)? {
            push_issue(issues, COMPOSITE);
        }
        return Ok(());
    }
    if !encoding_allowed(dict, doc)? {
        push_issue(issues, ENCODING);
    }
    if !simple_embedded(dict, doc)? {
        push_issue(issues, NOT_EMBEDDED);
    }
    Ok(())
}

fn check_annotation(dict: &PdfDictionary, issues: &mut Vec<String>) {
    let subtype = dict
        .get("Subtype")
        .and_then(|value| value.as_name())
        .map(str::to_string);
    if is_disallowed_annot(subtype.as_deref()) {
        push_issue(issues, ANNOT_FORBIDDEN);
    }
    if subtype.as_deref() == Some("Popup") {
        return;
    }
    let annot = is_annotation(dict);
    let appearance = has_appearance(dict);
    if annot && !appearance {
        push_issue(issues, APPEARANCE);
    }
}

fn metadata_present(doc: &mut PdfDocument, catalog: &PdfDictionary, part: u8) -> PdfResult<bool> {
    let Some(id) = catalog
        .get("Metadata")
        .and_then(|value| value.as_reference())
    else {
        return Ok(false);
    };
    let PdfObject::Stream(stream) = doc.get_object(id)? else {
        return Ok(false);
    };
    if stream.dict.contains_key("Filter") {
        return Ok(false);
    }
    Ok(metadata_matches(&stream.content, part))
}

fn metadata_matches(bytes: &[u8], part: u8) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let marker = format!(">{part}</pdfaid:part>");
    text.contains("http://www.aiim.org/pdfa/ns/id/")
        && text.contains(&marker)
        && text.contains("<pdfaid:conformance>B</pdfaid:conformance>")
}

fn output_intent_present(doc: &mut PdfDocument, catalog: &PdfDictionary) -> PdfResult<bool> {
    let Some(entries) = catalog
        .get("OutputIntents")
        .and_then(|value| value.as_array())
        .map(|items| items.to_vec())
    else {
        return Ok(false);
    };
    for item in entries {
        let Some(dict) = resolve_dict(&item, doc)? else {
            continue;
        };
        if dict.get("S").and_then(|value| value.as_name()) != Some("GTS_PDFA1") {
            continue;
        }
        let Some(profile_id) = dict
            .get("DestOutputProfile")
            .and_then(|value| value.as_reference())
        else {
            continue;
        };
        let PdfObject::Stream(stream) = doc.get_object(profile_id)? else {
            continue;
        };
        if stream.dict.contains_key("Filter") {
            continue;
        }
        if profile_bytes_match(&stream.content) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn profile_bytes_match(bytes: &[u8]) -> bool {
    if bytes.len() < 40 || &bytes[36..40] != b"acsp" {
        return false;
    }
    let declared = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    declared as usize == bytes.len()
}

fn trailer_id_present(doc: &PdfDocument) -> bool {
    let Some(array) = doc
        .xref
        .trailer
        .get("ID")
        .and_then(|value| value.as_array())
    else {
        return false;
    };
    array.len() == 2
        && array
            .iter()
            .all(|item| matches!(item, PdfObject::String(_)))
}

fn is_font_dict(dict: &PdfDictionary) -> bool {
    let type_name = dict.get("Type").and_then(|value| value.as_name());
    if type_name == Some("Font") {
        return true;
    }
    matches!(
        dict.get("Subtype").and_then(|value| value.as_name()),
        Some(
            "Type0"
                | "Type1"
                | "MMType1"
                | "Type3"
                | "TrueType"
                | "CIDFontType0"
                | "CIDFontType2"
                | "Type1C"
        )
    )
}

fn is_annotation(dict: &PdfDictionary) -> bool {
    if dict.get("Type").and_then(|value| value.as_name()) == Some("Annot") {
        return true;
    }
    known_annot(dict.get("Subtype").and_then(|value| value.as_name()))
}

fn known_annot(name: Option<&str>) -> bool {
    matches!(
        name,
        Some(
            "Text"
                | "Link"
                | "FreeText"
                | "Line"
                | "Square"
                | "Circle"
                | "Polygon"
                | "PolyLine"
                | "Highlight"
                | "Underline"
                | "StrikeOut"
                | "Squiggly"
                | "Stamp"
                | "Caret"
                | "Ink"
                | "Popup"
                | "FileAttachment"
                | "Widget"
                | "Screen"
                | "PrinterMark"
                | "TrapNet"
                | "Watermark"
                | "Sound"
                | "Movie"
                | "RichMedia"
                | "Redact"
        )
    )
}

fn is_disallowed_annot(name: Option<&str>) -> bool {
    matches!(
        name,
        Some("Sound" | "Movie" | "RichMedia" | "Screen" | "FileAttachment" | "TrapNet")
    )
}

fn is_structural(dict: &PdfDictionary) -> bool {
    let type_name = dict
        .get("Type")
        .and_then(|value| value.as_name())
        .map(str::to_string);
    let subtype = dict
        .get("Subtype")
        .and_then(|value| value.as_name())
        .map(str::to_string);
    structural_name(type_name.as_deref()) || structural_name(subtype.as_deref())
}

fn structural_name(name: Option<&str>) -> bool {
    matches!(
        name,
        Some(
            "Catalog"
                | "Pages"
                | "Page"
                | "Font"
                | "FontDescriptor"
                | "XObject"
                | "Metadata"
                | "OutputIntent"
                | "ObjStm"
                | "XRef"
                | "Annot"
                | "ExtGState"
                | "Encoding"
        )
    )
}
