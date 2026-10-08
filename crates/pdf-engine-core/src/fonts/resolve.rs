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

/// One simple or composite font resource, keyed later by the name used in `Tf` (no leading slash).
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
    /// Whether this font is a composite font (Type0 / CIDFont) with 2-byte character codes.
    pub is_composite: bool,
}

/// Collects font resources for `page_id` (both simple and composite Type0 fonts).
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
        .map(|(name, value)| {
            (
                name.as_str().trim_start_matches('/').to_string(),
                value.clone(),
            )
        })
        .collect();

    for (resource_name, value) in entries {
        if resource_name.is_empty() {
            continue;
        }
        let Some(font) = lookup_dict(doc, Some(value))? else {
            continue;
        };
        let subtype = font
            .get("Subtype")
            .and_then(|obj| obj.as_name())
            .unwrap_or("");
        if subtype == "Type0" {
            fonts.insert(resource_name, resolve_type0_font(doc, &font)?);
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

fn lookup_dict(
    doc: &mut PdfDocument,
    value: Option<PdfObject>,
) -> PdfResult<Option<PdfDictionary>> {
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
    let base = font
        .get("BaseFont")
        .and_then(|obj| obj.as_name())
        .unwrap_or("");
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
        is_composite: false,
    })
}

/// Maximum number of CIDs permitted when parsing `/W` to protect memory allocations.
const MAX_CID_COUNT: usize = 65536;

/// Parses CIDFont `/W` array into a map of `CID -> width` (ISO 32000-1 §9.7.4.3).
pub fn parse_cid_widths(w_array: &[PdfObject], max_cids: usize) -> BTreeMap<u32, f64> {
    let mut map = BTreeMap::new();
    let mut i = 0;
    while i < w_array.len() {
        let Some(first_cid_num) = w_array[i].as_i64() else {
            i += 1;
            continue;
        };
        let first_cid = first_cid_num.max(0) as u32;
        i += 1;
        if i >= w_array.len() {
            break;
        }
        match &w_array[i] {
            PdfObject::Array(widths) => {
                // Format 1: c [ w1 w2 ... wn ]
                for (offset, w_obj) in widths.iter().enumerate() {
                    if map.len() >= max_cids {
                        break;
                    }
                    if let Some(w) = w_obj.as_f64() {
                        let cid = first_cid.saturating_add(offset as u32);
                        map.insert(cid, w);
                    }
                }
                i += 1;
            }
            PdfObject::Integer(last_cid_num) => {
                // Format 2: c_first c_last w
                let last_cid = (*last_cid_num).max(0) as u32;
                i += 1;
                if i < w_array.len() {
                    if let Some(w) = w_array[i].as_f64() {
                        let start = first_cid.min(last_cid);
                        let end = first_cid.max(last_cid);
                        let count = (end.saturating_sub(start).saturating_add(1)) as usize;
                        let bounded_count = count.min(max_cids.saturating_sub(map.len()));
                        for offset in 0..bounded_count {
                            map.insert(start.saturating_add(offset as u32), w);
                        }
                    }
                    i += 1;
                }
            }
            PdfObject::Real(last_cid_num) => {
                let last_cid = (*last_cid_num).max(0.0) as u32;
                i += 1;
                if i < w_array.len() {
                    if let Some(w) = w_array[i].as_f64() {
                        let start = first_cid.min(last_cid);
                        let end = first_cid.max(last_cid);
                        let count = (end.saturating_sub(start).saturating_add(1)) as usize;
                        let bounded_count = count.min(max_cids.saturating_sub(map.len()));
                        for offset in 0..bounded_count {
                            map.insert(start.saturating_add(offset as u32), w);
                        }
                    }
                    i += 1;
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    map
}

fn read_array_objects(
    doc: &mut PdfDocument,
    value: Option<PdfObject>,
) -> PdfResult<Vec<PdfObject>> {
    let resolved = match value {
        Some(PdfObject::Reference(id)) => match doc.get_object(id) {
            Ok(obj) => obj,
            Err(PdfError::ObjectNotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        },
        Some(obj) => obj,
        None => return Ok(Vec::new()),
    };
    Ok(resolved.as_array().map(|s| s.to_vec()).unwrap_or_default())
}

fn resolve_type0_font(doc: &mut PdfDocument, font: &PdfDictionary) -> PdfResult<ResolvedFont> {
    let base = font
        .get("BaseFont")
        .and_then(|obj| obj.as_name())
        .unwrap_or("");

    let descendant_dict = match font.get("DescendantFonts") {
        Some(PdfObject::Array(arr)) => {
            if let Some(first) = arr.first() {
                lookup_dict(doc, Some(first.clone()))?
            } else {
                None
            }
        }
        Some(PdfObject::Reference(id)) => {
            if let Ok(PdfObject::Array(arr)) = doc.get_object(*id) {
                if let Some(first) = arr.first() {
                    lookup_dict(doc, Some(first.clone()))?
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };

    let effective_base = if !base.is_empty() {
        base
    } else if let Some(desc) = &descendant_dict {
        desc.get("BaseFont")
            .and_then(|obj| obj.as_name())
            .unwrap_or("")
    } else {
        ""
    };
    let (family, weight, style) = css_face(effective_base);

    let descriptor = if let Some(desc) = &descendant_dict {
        lookup_dict(doc, desc.get("FontDescriptor").cloned())?
    } else {
        lookup_dict(doc, font.get("FontDescriptor").cloned())?
    };

    let default_width = if let Some(desc) = &descendant_dict {
        desc.get("DW")
            .and_then(|obj| obj.as_f64())
            .unwrap_or(1000.0)
    } else {
        1000.0
    };

    let cid_widths = if let Some(desc) = &descendant_dict {
        let w_array = read_array_objects(doc, desc.get("W").cloned())?;
        parse_cid_widths(&w_array, MAX_CID_COUNT)
    } else {
        BTreeMap::new()
    };

    let mut metrics = FontMetrics::new_cid(cid_widths, default_width);
    if let Some(desc) = &descriptor {
        if let Some(ascent) = desc.get("Ascent").and_then(|obj| obj.as_f64()) {
            metrics.ascent = ascent;
        }
        if let Some(descent) = desc.get("Descent").and_then(|obj| obj.as_f64()) {
            metrics.descent = descent;
        }
    }

    let to_unicode_obj = font.get("ToUnicode").cloned().or_else(|| {
        descendant_dict
            .as_ref()
            .and_then(|d| d.get("ToUnicode").cloned())
    });
    let cmap = read_tounicode(doc, to_unicode_obj)?;

    Ok(ResolvedFont {
        metrics,
        cmap,
        family,
        weight,
        style,
        is_composite: true,
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

fn read_tounicode(
    doc: &mut PdfDocument,
    value: Option<PdfObject>,
) -> PdfResult<Option<ToUnicodeMap>> {
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
                    Some(PdfObject::Array(items)) => {
                        items.get(index).and_then(|item| item.as_dict())
                    }
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
        let face = fonts.get("F1").expect("F1");
        assert_eq!(face.family, "Arial");
        assert_eq!(face.style, "italic");
        assert_eq!(face.weight, 400);
        assert!(!face.is_composite);
        assert!(face.cmap.is_none());

        let c0 = fonts.get("C0").expect("C0");
        assert!(c0.is_composite);
        assert_eq!(c0.family, "CIDFont");
        assert!((c0.metrics.default_width - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn resolves_type0_composite_font_with_cid_widths_and_tounicode() {
        let mut doc = PdfDocument::empty();
        let cmap_id = ObjectId::new(30);
        let descendant_id = ObjectId::new(31);
        let type0_id = ObjectId::new(32);
        let page_id = ObjectId::new(33);

        let cmap_raw = b"2 beginbfchar\n<0001> <0041>\n<0002> <0042>\nendbfchar\n";
        let mut cmap_dict = PdfDictionary::new();
        cmap_dict.insert("Length", cmap_raw.len() as i64);
        doc.set_object(
            cmap_id,
            PdfObject::Stream(PdfStream::new(cmap_dict, cmap_raw.to_vec())),
        );

        let mut desc = PdfDictionary::new();
        desc.insert("Type", PdfName::new("Font"));
        desc.insert("Subtype", PdfName::new("CIDFontType2"));
        desc.insert("BaseFont", PdfName::new("ABCDEF+Calibri-Bold"));
        desc.insert("DW", 1000i64);
        desc.insert(
            "W",
            vec![
                PdfObject::Integer(1),
                PdfObject::Array(vec![PdfObject::Integer(500), PdfObject::Integer(600)]),
                PdfObject::Integer(10),
                PdfObject::Integer(12),
                PdfObject::Integer(750),
            ],
        );
        let mut desc_descriptor = PdfDictionary::new();
        desc_descriptor.insert("Ascent", 750i64);
        desc_descriptor.insert("Descent", -250i64);
        desc.insert("FontDescriptor", PdfObject::Dictionary(desc_descriptor));
        doc.set_object(descendant_id, PdfObject::Dictionary(desc));

        let mut type0 = PdfDictionary::new();
        type0.insert("Type", PdfName::new("Font"));
        type0.insert("Subtype", PdfName::new("Type0"));
        type0.insert("BaseFont", PdfName::new("ABCDEF+Calibri-Bold"));
        type0.insert("Encoding", PdfName::new("Identity-H"));
        type0.insert("DescendantFonts", vec![PdfObject::Reference(descendant_id)]);
        type0.insert("ToUnicode", cmap_id);
        doc.set_object(type0_id, PdfObject::Dictionary(type0));

        let mut resources = PdfDictionary::new();
        let mut font_dict = PdfDictionary::new();
        font_dict.insert("C0", type0_id);
        resources.insert("Font", PdfObject::Dictionary(font_dict));

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfName::new("Page"));
        page.insert("Resources", PdfObject::Dictionary(resources));
        doc.set_object(page_id, PdfObject::Dictionary(page));

        let fonts = resolve_page_fonts(&mut doc, page_id).expect("fonts");
        let face = fonts.get("C0").expect("C0");
        assert!(face.is_composite);
        assert_eq!(face.family, "Calibri-Bold");
        assert_eq!(face.weight, 700);
        assert_eq!(face.style, "normal");

        assert!((face.metrics.get_glyph_width(1) - 500.0).abs() < 1e-6);
        assert!((face.metrics.get_glyph_width(2) - 600.0).abs() < 1e-6);
        assert!((face.metrics.get_glyph_width(10) - 750.0).abs() < 1e-6);
        assert!((face.metrics.get_glyph_width(11) - 750.0).abs() < 1e-6);
        assert!((face.metrics.get_glyph_width(12) - 750.0).abs() < 1e-6);
        assert!((face.metrics.get_glyph_width(99) - 1000.0).abs() < 1e-6);

        let cmap = face.cmap.as_ref().expect("cmap");
        assert_eq!(cmap.decode_code(1), Some("A"));
        assert_eq!(cmap.decode_code(2), Some("B"));
    }
}
