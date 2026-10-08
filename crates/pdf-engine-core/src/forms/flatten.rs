//! High-performance surgical form flattening engine.
//!
//! Converts interactive AcroForm fields into permanent page vector graphics and text,
//! strips widget annotations, and removes interactive forms to prevent document tampering.

use crate::cos::object::{ObjectId, PdfDictionary, PdfName, PdfObject, PdfStream};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::forms::reader::extract_document_forms;
use crate::forms::types::{FormField, FormFieldType};
use std::collections::HashMap;

/// Flattens all interactive form fields across the entire document into permanent vector graphics and text.
///
/// Returns the number of fields successfully flattened.
pub fn flatten_document_forms(doc: &mut PdfDocument) -> PdfResult<usize> {
    let fields = extract_document_forms(doc)?;
    if fields.is_empty() {
        return Ok(0);
    }

    let field_count = fields.len();

    // Group fields by page_id
    let mut fields_by_page: HashMap<ObjectId, Vec<FormField>> = HashMap::new();
    for field in fields {
        fields_by_page.entry(field.page_id).or_default().push(field);
    }

    for (page_id, page_fields) in fields_by_page {
        flatten_page_fields(doc, page_id, &page_fields)?;
    }

    // Neutralize /AcroForm in the document Catalog
    remove_acroform_from_catalog(doc)?;

    Ok(field_count)
}

/// Flattens fields for a single page, appending visual operations to /Contents and removing /Annots widgets.
fn flatten_page_fields(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    fields: &[FormField],
) -> PdfResult<()> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(()),
    };

    // 1. Build graphics operations for all visible field values
    let mut append_ops = String::from("\n% --- Flattened AcroForm Fields ---\n");
    let mut needs_helv_font = false;

    for field in fields {
        let width = field.rect.width().max(5.0);
        let height = field.rect.height().max(5.0);
        let llx = field.rect.min_x;
        let lly = field.rect.min_y;

        match field.field_type {
            FormFieldType::Text | FormFieldType::Choice => {
                if !field.value.is_empty() {
                    needs_helv_font = true;
                    let font_size = (height * 0.65).clamp(8.0, 14.0);
                    let baseline_y = (height - font_size) * 0.5;
                    let escaped = escape_pdf_string(&field.value);

                    append_ops.push_str(&format!(
                        "q\nBT\n/F1 {:.1} Tf\n0 0 0 rg\n{:.2} {:.2} Td\n({}) Tj\nET\nQ\n",
                        font_size,
                        llx + 2.0,
                        lly + baseline_y,
                        escaped
                    ));
                }
            }

            FormFieldType::Checkbox => {
                if field.is_checked() {
                    append_ops.push_str(&format!(
                        "q\n0 0 0 RG\n1.8 w\n{:.2} {:.2} m\n{:.2} {:.2} l\n{:.2} {:.2} l\nS\nQ\n",
                        llx + width * 0.2,
                        lly + height * 0.5,
                        llx + width * 0.45,
                        lly + height * 0.25,
                        llx + width * 0.8,
                        lly + height * 0.75
                    ));
                }
            }

            FormFieldType::RadioButton => {
                if field.is_checked() {
                    // Draw a filled dot inside the radio button box
                    let center_x = llx + width * 0.5;
                    let center_y = lly + height * 0.5;
                    let radius = (width.min(height) * 0.25).max(2.0);
                    append_ops.push_str(&format!(
                        "q\n0 0 0 rg\n{:.2} {:.2} {:.2} {:.2} re\nf\nQ\n",
                        center_x - radius,
                        center_y - radius,
                        radius * 2.0,
                        radius * 2.0
                    ));
                }
            }

            _ => {}
        }
    }

    // 2. Append new operations to page Content Stream
    let mut existing_bytes = doc.get_page_content_bytes(page_id).unwrap_or_default();
    existing_bytes.extend_from_slice(append_ops.as_bytes());

    // Update /Contents
    let new_contents_stream_id = doc.alloc_object_id();
    let mut stream_dict = PdfDictionary::new();
    stream_dict.insert("Length", existing_bytes.len() as i64);
    let new_stream = PdfStream::new(stream_dict, existing_bytes);
    doc.set_object(new_contents_stream_id, PdfObject::Stream(new_stream));

    page_dict.insert("Contents", PdfObject::Reference(new_contents_stream_id));

    // 3. Ensure page has /F1 in /Resources /Font if needed
    if needs_helv_font {
        ensure_page_font_resource(doc, &mut page_dict)?;
    }

    // 4. Strip Widget annotations associated with these fields from /Annots
    let field_ids: std::collections::HashSet<ObjectId> = fields.iter().map(|f| f.id).collect();

    if let Some(annots_obj) = page_dict.get_mut("Annots") {
        if let Some(annots_arr) = annots_obj.as_array_mut() {
            annots_arr.retain(|item| {
                if let Some(r) = item.as_reference() {
                    !field_ids.contains(&r)
                } else {
                    true
                }
            });
        }
    }

    // If /Annots is now empty, remove it
    let annots_is_empty = page_dict
        .get("Annots")
        .and_then(|a| a.as_array())
        .map(|arr| arr.is_empty())
        .unwrap_or(false);

    if annots_is_empty {
        page_dict.remove("Annots");
    }

    doc.set_object(page_id, PdfObject::Dictionary(page_dict));
    Ok(())
}

/// Ensures the page /Resources /Font dictionary contains /F1 (Helvetica).
fn ensure_page_font_resource(
    doc: &mut PdfDocument,
    page_dict: &mut PdfDictionary,
) -> PdfResult<()> {
    let mut resources = match page_dict.get("Resources") {
        Some(PdfObject::Dictionary(d)) => d.clone(),
        Some(PdfObject::Reference(r)) => match doc.get_object(*r)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    let mut fonts = match resources.get("Font") {
        Some(PdfObject::Dictionary(d)) => d.clone(),
        Some(PdfObject::Reference(r)) => match doc.get_object(*r)? {
            PdfObject::Dictionary(d) => d,
            _ => PdfDictionary::new(),
        },
        _ => PdfDictionary::new(),
    };

    if !fonts.contains_key("F1") {
        let mut helv = PdfDictionary::new();
        helv.insert("Type", PdfObject::Name(PdfName::new("Font")));
        helv.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
        helv.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
        fonts.insert("F1", PdfObject::Dictionary(helv));
    }

    resources.insert("Font", PdfObject::Dictionary(fonts));
    page_dict.insert("Resources", PdfObject::Dictionary(resources));
    Ok(())
}

/// Removes the `/AcroForm` entry from the document Catalog.
fn remove_acroform_from_catalog(doc: &mut PdfDocument) -> PdfResult<()> {
    let root_ref = doc
        .xref
        .trailer
        .get("Root")
        .and_then(|r| r.as_reference())
        .ok_or_else(|| PdfError::InvalidXRef {
            offset: 0,
            message: "Missing trailer /Root".to_string(),
        })?;

    let root_obj = doc.get_object(root_ref)?;
    if let PdfObject::Dictionary(mut catalog) = root_obj {
        catalog.remove("AcroForm");
        doc.set_object(root_ref, PdfObject::Dictionary(catalog));
    }

    Ok(())
}

/// Escapes characters in PDF strings.
fn escape_pdf_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}
