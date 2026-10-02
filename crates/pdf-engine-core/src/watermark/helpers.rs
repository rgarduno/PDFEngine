//! Low-level helpers for page content streams, resources, and geometric boundaries.

use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream};
use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::layout::geometry::Rect;

/// Extracts the MediaBox bounding box of a page, falling back to Parent node or standard US Letter (612x792).
pub fn get_page_mediabox(doc: &mut PdfDocument, page_id: ObjectId) -> Rect {
    if let Ok(PdfObject::Dictionary(d)) = doc.get_object(page_id) {
        if let Some(PdfObject::Array(arr)) = d.get("MediaBox") {
            if arr.len() == 4 {
                let x0 = arr[0].as_f64().unwrap_or(0.0);
                let y0 = arr[1].as_f64().unwrap_or(0.0);
                let x1 = arr[2].as_f64().unwrap_or(612.0);
                let y1 = arr[3].as_f64().unwrap_or(792.0);
                return Rect::new(x0, y0, x1, y1);
            }
        }
        if let Some(PdfObject::Reference(parent_id)) = d.get("Parent") {
            if let Ok(PdfObject::Dictionary(p_dict)) = doc.get_object(*parent_id) {
                if let Some(PdfObject::Array(arr)) = p_dict.get("MediaBox") {
                    if arr.len() == 4 {
                        let x0 = arr[0].as_f64().unwrap_or(0.0);
                        let y0 = arr[1].as_f64().unwrap_or(0.0);
                        let x1 = arr[2].as_f64().unwrap_or(612.0);
                        let y1 = arr[3].as_f64().unwrap_or(792.0);
                        return Rect::new(x0, y0, x1, y1);
                    }
                }
            }
        }
    }
    Rect::new(0.0, 0.0, 612.0, 792.0)
}

/// Escapes special characters for PDF literal strings `(...)`.
pub fn escape_pdf(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Injects content operations at the beginning of a page stream (Background layer).
pub fn prepend_content_ops(doc: &mut PdfDocument, page_id: ObjectId, ops: &str) -> PdfResult<()> {
    let existing_bytes = doc.get_page_content_bytes(page_id).unwrap_or_default();
    let mut new_bytes = Vec::with_capacity(ops.len() + existing_bytes.len() + 2);
    new_bytes.extend_from_slice(ops.as_bytes());
    if !ops.ends_with('\n') {
        new_bytes.push(b'\n');
    }
    new_bytes.extend_from_slice(&existing_bytes);

    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(()),
    };

    let new_contents_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Length", PdfObject::Integer(new_bytes.len() as i64));
    let stream = PdfStream::new(stream_dict, new_bytes);
    doc.set_object(new_contents_id, PdfObject::Stream(stream));

    page_dict.insert("Contents", PdfObject::Reference(new_contents_id));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(())
}

/// Injects content operations at the end of a page stream (Foreground layer).
pub fn append_content_ops(doc: &mut PdfDocument, page_id: ObjectId, ops: &str) -> PdfResult<()> {
    let mut existing_bytes = doc.get_page_content_bytes(page_id).unwrap_or_default();
    if !existing_bytes.is_empty() && !existing_bytes.ends_with(b"\n") {
        existing_bytes.push(b'\n');
    }
    existing_bytes.extend_from_slice(ops.as_bytes());

    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(()),
    };

    let new_contents_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Length", PdfObject::Integer(existing_bytes.len() as i64));
    let stream = PdfStream::new(stream_dict, existing_bytes);
    doc.set_object(new_contents_id, PdfObject::Stream(stream));

    page_dict.insert("Contents", PdfObject::Reference(new_contents_id));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(())
}

/// Ensures a Standard 14 Type 1 font is available in `/Resources /Font` for the given page.
pub fn ensure_font_resource(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    font_alias: &str,
    font_name: &str,
) -> PdfResult<()> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(()),
    };

    let mut res_dict = match page_dict.remove("Resources") {
        Some(PdfObject::Dictionary(d)) => d,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    let mut font_dict = match res_dict.remove("Font") {
        Some(PdfObject::Dictionary(d)) => d,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    if !font_dict.contains_key(font_alias) {
        let mut f_dict = PdfDictionary::new();
        f_dict.insert("Type", PdfObject::Name("Font".into()));
        f_dict.insert("Subtype", PdfObject::Name("Type1".into()));
        f_dict.insert("BaseFont", PdfObject::Name(font_name.into()));
        font_dict.insert(font_alias, PdfObject::Dictionary(f_dict));
    }

    res_dict.insert("Font", PdfObject::Dictionary(font_dict));
    page_dict.insert("Resources", PdfObject::Dictionary(res_dict));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(())
}

/// Ensures an ExtGState with opacity settings is registered in `/Resources /ExtGState` for the given page.
pub fn ensure_extgstate_resource(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    gs_alias: &str,
    opacity: f64,
) -> PdfResult<()> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(()),
    };

    let mut res_dict = match page_dict.remove("Resources") {
        Some(PdfObject::Dictionary(d)) => d,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    let mut extg_dict = match res_dict.remove("ExtGState") {
        Some(PdfObject::Dictionary(d)) => d,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    let alpha = opacity.clamp(0.01, 1.0);
    let mut gs = PdfDictionary::new();
    gs.insert("Type", PdfObject::Name("ExtGState".into()));
    gs.insert("ca", PdfObject::Real(alpha));
    gs.insert("CA", PdfObject::Real(alpha));
    extg_dict.insert(gs_alias, PdfObject::Dictionary(gs));

    res_dict.insert("ExtGState", PdfObject::Dictionary(extg_dict));
    page_dict.insert("Resources", PdfObject::Dictionary(res_dict));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(())
}

/// Ensures an Image XObject is registered in `/Resources /XObject` for the given page.
pub fn ensure_xobject_resource(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    xobj_alias: &str,
    xobj_id: ObjectId,
) -> PdfResult<()> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(()),
    };

    let mut res_dict = match page_dict.remove("Resources") {
        Some(PdfObject::Dictionary(d)) => d,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    let mut xobj_dict = match res_dict.remove("XObject") {
        Some(PdfObject::Dictionary(d)) => d,
        Some(PdfObject::Reference(id)) => match doc.get_object(id)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    xobj_dict.insert(xobj_alias, PdfObject::Reference(xobj_id));

    res_dict.insert("XObject", PdfObject::Dictionary(xobj_dict));
    page_dict.insert("Resources", PdfObject::Dictionary(res_dict));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(())
}
