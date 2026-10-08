//! Resolves a page's `/Font` resources into metrics, `/ToUnicode` maps, and a CSS face.
//!
//! Simple fonts (`/TrueType`, `/Type1`, `/Type3`) are read from the page dictionary
//! or, when `/Resources` is inherited, from an ancestor `/Pages` node. Composite
//! `/Type0` fonts are left untouched: their character codes are not single bytes.

use std::collections::{BTreeMap, HashSet};

use crate::cos::{decode_stream, ObjectId, PdfDictionary, PdfDocument, PdfObject, PdfStream};
use crate::error::{PdfError, PdfResult};
use crate::fonts::metrics::FontMetrics;
use crate::fonts::tounicode::ToUnicodeMap;

/// How many `/Parent` links are followed when a page inherits `/Resources`.
const MAX_PARENT_HOPS: usize = 16;

/// One simple font resource, keyed later by the name used in `Tf` (no leading slash).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFont {
    /// Advance widths from `/Widths`, or the descriptor `/MissingWidth` when absent.
    pub metrics: FontMetrics,
    /// `/ToUnicode` CMap. Absent when the font has no usable map.
    pub cmap: Option<ToUnicodeMap>,
    /// CSS family name. Empty when `/BaseFont` does not name a known face.
    pub family: String,
    /// 700 when the base name contains Bold, otherwise 400.
    pub weight: u16,
    /// `"italic"` or `"normal"`.
    pub style: String,
}

/// Collects simple font resources for `page_id`.
///
/// A missing font dictionary yields an empty map. A security-limit failure
/// while inflating `/ToUnicode` is returned to the caller.
pub fn resolve_page_fonts(
    doc: &mut PdfDocument,
    page_id: ObjectId,
) -> PdfResult<BTreeMap<String, ResolvedFont>> {
    let mut fonts = BTreeMap::new();
    let Some(resources) = page_resources(doc, page_id)? else {
        return Ok(fonts);
    };
    let Some(font_dict) = lookup_dict(doc, resources.get("Font").cloned())? else {
        return Ok(fonts);
    };

    let entries: Vec<(String, PdfObject)> = font_dict
        .0
        .iter()
        .map(|(name, value)| (name.as_str().trim_start_matches('/').to_string(), value.clone()))
        .collect();

    for (resource_name, value) in entries {
        if resource_name.is_empty() {
            continue;
        }
        let Some(font) = lookup_dict(doc, Some(value))? else {
            continue;
        };
        let subtype = font.get("Subtype").and_then(|obj| obj.as_name()).unwrap_or("");
        if subtype == "Type0" {
            continue;
        }
        fonts.insert(resource_name, resolve_simple_font(doc, &font)?);
    }

    Ok(fonts)
}

/// Maps a PDF `/BaseFont` (subset prefix included) to a CSS family, weight, and style.
pub fn css_face(base_font: &str) -> (String, u16, String) {
    let bare = base_font.rsplit('+').next().unwrap_or(base_font);
    let lower = bare.to_ascii_lowercase();
    let family = if lower.contains("times") {
        "Times New Roman".to_string()
    } else if lower.contains("arial") || lower.contains("helvetica") {
        "Arial".to_string()
    } else if lower.contains("courier") {
        "Courier New".to_string()
    } else if bare.is_empty() {
        String::new()
    } else {
        bare.to_string()
    };
    let weight = if lower.contains("bold") || lower.contains("black") || lower.contains("heavy") {
        700
    } else {
        400
    };
    let style = if lower.contains("italic") || lower.contains("oblique") {
        "italic"
    } else {
        "normal"
    };
    (family, weight, style.to_string())
}

fn page_resources(doc: &mut PdfDocument, page_id: ObjectId) -> PdfResult<Option<PdfDictionary>> {
    let mut current = page_id;
    let mut seen = HashSet::new();
    for _ in 0..MAX_PARENT_HOPS {
        if !seen.insert(current) {
            return Ok(None);
        }
        let dict = match doc.get_object(current) {
            Ok(PdfObject::Dictionary(dict)) => dict.clone(),
            Ok(_) => return Ok(None),
            Err(PdfError::ObjectNotFound { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Some(resources) = lookup_dict(doc, dict.get("Resources").cloned())? {
            return Ok(Some(resources));
        }
        match dict.get("Parent").and_then(|parent| parent.as_reference()) {
            Some(parent) => current = parent,
            None => return Ok(None),
        }
    }
    Ok(None)
}

fn lookup_dict(doc: &mut PdfDocument, value: Option<PdfObject>) -> PdfResult<Option<PdfDictionary>> {
    match value {
        Some(PdfObject::Dictionary(dict)) => Ok(Some(dict)),
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(PdfObject::Dictionary(dict)) => Ok(Some(dict)),
            Ok(_) => Ok(None),
            Err(PdfError::ObjectNotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        },
        _ => Ok(None),
    }
}

fn resolve_simple_font(doc: &mut PdfDocument, font: &PdfDictionary) -> PdfResult<ResolvedFont> {
    let base = font.get("BaseFont").and_then(|obj| obj.as_name()).unwrap_or("");
    let (family, weight, style) = css_face(base);
    let descriptor = lookup_dict(doc, font.get("FontDescriptor").cloned())?;
    let missing = descriptor
        .as_ref()
        .and_then(|dict| dict.get("MissingWidth"))
        .and_then(|obj| obj.as_f64())
        .unwrap_or(500.0);
    let first = font
        .get("FirstChar")
        .and_then(|obj| obj.as_f64())
        .map(|value| value as u32)
        .unwrap_or(0);
    let last = font
        .get("LastChar")
        .and_then(|obj| obj.as_f64())
        .map(|value| value as u32)
        .unwrap_or(255);
    let widths = read_widths(doc, font.get("Widths").cloned())?;
    let mut metrics = FontMetrics::new(first, last, widths, missing);
    if let Some(descriptor) = &descriptor {
        if let Some(ascent) = descriptor.get("Ascent").and_then(|obj| obj.as_f64()) {
            metrics.ascent = ascent;
        }
        if let Some(descent) = descriptor.get("Descent").and_then(|obj| obj.as_f64()) {
            metrics.descent = descent;
        }
    }
    let cmap = read_tounicode(doc, font.get("ToUnicode").cloned())?;
    Ok(ResolvedFont {
        metrics,
        cmap,
        family,
        weight,
        style,
    })
}

fn read_widths(doc: &mut PdfDocument, value: Option<PdfObject>) -> PdfResult<Vec<f64>> {
    let resolved = match value {
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(obj) => obj,
            Err(PdfError::ObjectNotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        },
        Some(obj) => obj,
        None => return Ok(Vec::new()),
    };
    Ok(resolved
        .as_array()
        .map(|items| items.iter().filter_map(|item| item.as_f64()).collect())
        .unwrap_or_default())
}

fn read_tounicode(doc: &mut PdfDocument, value: Option<PdfObject>) -> PdfResult<Option<ToUnicodeMap>> {
    let resolved = match value {
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(obj) => obj,
            Err(PdfError::ObjectNotFound { .. }) => return Ok(None),
            Err(error) => return Err(error),
        },
        Some(obj) => obj,
        None => return Ok(None),
    };
    let PdfObject::Stream(stream) = resolved else {
        return Ok(None);
    };
    let bytes = inflate_stream(doc, &stream)?;
    match ToUnicodeMap::parse(&bytes) {
        Ok(map) if !map.char_to_unicode.is_empty() => Ok(Some(map)),
        _ => Ok(None),
    }
}

fn inflate_stream(doc: &PdfDocument, stream: &PdfStream) -> PdfResult<Vec<u8>> {
    let limits = &doc.limits;
    match stream.dict.get("Filter") {
        None => decode_stream("", None, &stream.content, limits),
        Some(PdfObject::Name(name)) => {
            let params = stream.dict.get("DecodeParms").and_then(|obj| obj.as_dict());
            decode_stream(name.as_str(), params, &stream.content, limits)
        }
        Some(PdfObject::Array(filters)) => {
            let mut data = stream.content.clone();
            let parms = stream.dict.get("DecodeParms").cloned();
            for (index, filter) in filters.iter().enumerate() {
                let name = filter.as_name().unwrap_or("");
                let params = match &parms {
                    Some(PdfObject::Array(items)) => items.get(index).and_then(|item| item.as_dict()),
                    Some(PdfObject::Dictionary(dict)) if index == 0 => Some(dict),
                    _ => None,
                };
                data = decode_stream(name, params, &data, limits)?;
            }
            Ok(data)
        }
        Some(_) => decode_stream("", None, &stream.content, limits),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::{ObjectId, PdfDocument, PdfName, PdfStream};

    fn cmap_bytes() -> Vec<u8> {
        b"19 beginbfrange\n<21> <21> <0052>\n<22> <22> <0061>\n<2F> <2F> <00F1>\nendbfrange\n"
            .to_vec()
    }

    #[test]
    fn resolves_tounicode_widths_and_css_face() {
        let mut doc = PdfDocument::empty();
        let cmap_id = ObjectId::new(10);
        let font_id = ObjectId::new(11);
        let page_id = ObjectId::new(12);

        let mut cmap_dict = PdfDictionary::new();
        cmap_dict.insert("Length", cmap_bytes().len() as i64);
        doc.set_object(
            cmap_id,
            PdfObject::Stream(PdfStream::new(cmap_dict, cmap_bytes())),
        );

        let mut font = PdfDictionary::new();
        font.insert("Type", PdfName::new("Font"));
        font.insert("Subtype", PdfName::new("TrueType"));
        font.insert("BaseFont", PdfName::new("AAAAAC+TimesNewRomanPS-BoldMT"));
        font.insert("FirstChar", 33i64);
        font.insert("LastChar", 47i64);
        font.insert(
            "Widths",
            vec![PdfObject::Integer(600), PdfObject::Integer(400)],
        );
        font.insert("ToUnicode", cmap_id);
        let mut descriptor = PdfDictionary::new();
        descriptor.insert("MissingWidth", 250i64);
        descriptor.insert("Ascent", 891i64);
        font.insert("FontDescriptor", PdfObject::Dictionary(descriptor));
        doc.set_object(font_id, PdfObject::Dictionary(font));

        let mut resources = PdfDictionary::new();
        let mut font_dict = PdfDictionary::new();
        font_dict.insert("TT2", font_id);
        resources.insert("Font", PdfObject::Dictionary(font_dict));

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfName::new("Page"));
        page.insert("Resources", PdfObject::Dictionary(resources));
        doc.set_object(page_id, PdfObject::Dictionary(page));

        let fonts = resolve_page_fonts(&mut doc, page_id).expect("fonts");
        let face = fonts.get("TT2").expect("TT2");
        assert_eq!(face.family, "Times New Roman");
        assert_eq!(face.weight, 700);
        assert_eq!(face.style, "normal");
        assert!((face.metrics.get_glyph_width(33) - 600.0).abs() < 1e-6);
        assert!((face.metrics.get_glyph_width(40) - 250.0).abs() < 1e-6);
        assert!((face.metrics.ascent - 891.0).abs() < 1e-6);
        let cmap = face.cmap.as_ref().expect("cmap");
        assert_eq!(cmap.decode_code(0x21), Some("R"));
        assert_eq!(cmap.decode_code(0x22), Some("a"));
        assert_eq!(cmap.decode_code(0x2F), Some("ñ"));
    }

    #[test]
    fn inherits_resources_from_parent_and_skips_type0() {
        let mut doc = PdfDocument::empty();
        let parent_id = ObjectId::new(20);
        let page_id = ObjectId::new(21);
        let simple_id = ObjectId::new(22);
        let composite_id = ObjectId::new(23);

        let mut simple = PdfDictionary::new();
        simple.insert("Subtype", PdfName::new("Type1"));
        simple.insert("BaseFont", PdfName::new("Arial-ItalicMT"));
        doc.set_object(simple_id, PdfObject::Dictionary(simple));

        let mut composite = PdfDictionary::new();
        composite.insert("Subtype", PdfName::new("Type0"));
        composite.insert("BaseFont", PdfName::new("AAAAAA+CIDFont"));
        doc.set_object(composite_id, PdfObject::Dictionary(composite));

        let mut font_dict = PdfDictionary::new();
        font_dict.insert("F1", simple_id);
        font_dict.insert("C0", composite_id);
        let mut resources = PdfDictionary::new();
        resources.insert("Font", PdfObject::Dictionary(font_dict));
        let mut parent = PdfDictionary::new();
        parent.insert("Type", PdfName::new("Pages"));
        parent.insert("Resources", PdfObject::Dictionary(resources));
        doc.set_object(parent_id, PdfObject::Dictionary(parent));

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfName::new("Page"));
        page.insert("Parent", parent_id);
        doc.set_object(page_id, PdfObject::Dictionary(page));

        let fonts = resolve_page_fonts(&mut doc, page_id).expect("fonts");
        assert!(fonts.get("C0").is_none());
        let face = fonts.get("F1").expect("F1");
        assert_eq!(face.family, "Arial");
        assert_eq!(face.style, "italic");
        assert_eq!(face.weight, 400);
        assert!(face.cmap.is_none());
    }
}
