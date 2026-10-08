//! Logic for setting AcroForm field values and synthesizing appearance streams (/AP /N).

use crate::cos::object::{
    ObjectId, PdfArray, PdfDictionary, PdfName, PdfObject, PdfStream, PdfString,
};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::forms::reader::extract_document_forms;
use crate::forms::types::{FormField, FormFieldType};
use std::collections::HashMap;

/// Fills a single form field by name or ID and updates its visual appearance stream.
pub fn fill_field_value(
    doc: &mut PdfDocument,
    field_identifier: &str,
    new_value: &str,
) -> PdfResult<bool> {
    let fields = extract_document_forms(doc)?;
    let target = fields
        .iter()
        .find(|f| f.name == field_identifier || f.id.number.to_string() == field_identifier);

    let target_field = match target {
        Some(f) => f.clone(),
        None => return Ok(false),
    };

    apply_field_value_and_appearance(doc, &target_field, new_value)?;
    Ok(true)
}

/// Fills multiple form fields in a single batch pass.
/// The `values` map maps field names to their desired string values.
pub fn fill_fields_batch(
    doc: &mut PdfDocument,
    values: &HashMap<String, String>,
) -> PdfResult<usize> {
    let fields = extract_document_forms(doc)?;
    let mut updated_count = 0;

    for field in fields {
        if let Some(new_val) = values
            .get(&field.name)
            .or_else(|| values.get(&field.id.number.to_string()))
        {
            apply_field_value_and_appearance(doc, &field, new_val)?;
            updated_count += 1;
        }
    }

    Ok(updated_count)
}

/// Sets the internal dictionary keys and synthesizes appearance streams for a form field.
fn apply_field_value_and_appearance(
    doc: &mut PdfDocument,
    field: &FormField,
    new_value: &str,
) -> PdfResult<()> {
    let mut field_dict = match doc.get_object(field.id)? {
        PdfObject::Dictionary(d) => d,
        _ => {
            return Err(PdfError::ObjectNotFound {
                id: field.id.number,
                gen: field.id.generation,
            })
        }
    };

    let width = field.rect.width().max(10.0);
    let height = field.rect.height().max(10.0);

    match field.field_type {
        FormFieldType::Text | FormFieldType::Choice => {
            // Update /V value
            field_dict.insert(
                "V",
                PdfObject::String(PdfString::literal(new_value.as_bytes().to_vec())),
            );

            // Synthesize Form XObject for /AP /N (Normal Appearance)
            let font_size = (height * 0.65).clamp(8.0, 14.0);
            let baseline_y = (height - font_size) * 0.5;
            let escaped_text = escape_pdf_string(new_value);

            let stream_ops = format!(
                "/Tx BMC\nq\nBT\n/Helv {:.1} Tf\n0 0 0 rg\n2.0 {:.1} Td\n({}) Tj\nET\nQ\nEMC\n",
                font_size, baseline_y, escaped_text
            );

            let appearance_stream_id =
                create_form_xobject_stream(doc, width, height, stream_ops.into_bytes())?;

            // Link into /AP << /N {stream_ref} >>
            let mut ap_dict = PdfDictionary::new();
            ap_dict.insert("N", PdfObject::Reference(appearance_stream_id));
            field_dict.insert("AP", PdfObject::Dictionary(ap_dict));
        }

        FormFieldType::Checkbox => {
            let is_checked = {
                let v = new_value.trim().to_lowercase();
                v == "yes" || v == "true" || v == "1" || v == "on"
            };

            let state_name = if is_checked { "Yes" } else { "Off" };
            field_dict.insert("V", PdfObject::Name(PdfName::new(state_name)));
            field_dict.insert("AS", PdfObject::Name(PdfName::new(state_name)));

            // Synthesize appearance for checkbox
            let stream_ops =
                if is_checked {
                    format!(
                    "q\n0.2 0.2 0.2 rg\n1.8 w\n{:.1} {:.1} m\n{:.1} {:.1} l\n{:.1} {:.1} l\nS\nQ\n",
                    width * 0.2, height * 0.5,
                    width * 0.45, height * 0.25,
                    width * 0.8, height * 0.75
                )
                } else {
                    "q\nQ\n".to_string()
                };

            let appearance_stream_id =
                create_form_xobject_stream(doc, width, height, stream_ops.into_bytes())?;

            let mut ap_dict = PdfDictionary::new();
            ap_dict.insert("N", PdfObject::Reference(appearance_stream_id));
            field_dict.insert("AP", PdfObject::Dictionary(ap_dict));
        }

        FormFieldType::RadioButton => {
            let state_name = if new_value.trim().is_empty() || new_value.trim() == "Off" {
                "Off"
            } else {
                new_value
            };
            field_dict.insert("V", PdfObject::Name(PdfName::new(state_name)));
            field_dict.insert("AS", PdfObject::Name(PdfName::new(state_name)));
        }

        _ => {
            field_dict.insert(
                "V",
                PdfObject::String(PdfString::literal(new_value.as_bytes().to_vec())),
            );
        }
    }

    doc.set_object(field.id, PdfObject::Dictionary(field_dict));
    Ok(())
}

/// Creates an indirect Form XObject stream suitable for `/AP /N` appearance streams.
fn create_form_xobject_stream(
    doc: &mut PdfDocument,
    width: f64,
    height: f64,
    stream_bytes: Vec<u8>,
) -> PdfResult<ObjectId> {
    let stream_id = doc.alloc_object_id();

    let mut dict = PdfDictionary::new();
    dict.insert("Type", PdfObject::Name(PdfName::new("XObject")));
    dict.insert("Subtype", PdfObject::Name(PdfName::new("Form")));

    let mut bbox = PdfArray::new();
    bbox.push(PdfObject::Real(0.0));
    bbox.push(PdfObject::Real(0.0));
    bbox.push(PdfObject::Real(width));
    bbox.push(PdfObject::Real(height));
    dict.insert("BBox", PdfObject::Array(bbox));
    dict.insert("Length", stream_bytes.len() as i64);

    // Standard font resources for the appearance stream
    let mut font_dict = PdfDictionary::new();
    let mut helv_dict = PdfDictionary::new();
    helv_dict.insert("Type", PdfObject::Name(PdfName::new("Font")));
    helv_dict.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
    helv_dict.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
    font_dict.insert("Helv", PdfObject::Dictionary(helv_dict));

    let mut res_dict = PdfDictionary::new();
    res_dict.insert("Font", PdfObject::Dictionary(font_dict));
    dict.insert("Resources", PdfObject::Dictionary(res_dict));

    let stream = PdfStream::new(dict, stream_bytes);
    doc.set_object(stream_id, PdfObject::Stream(stream));

    Ok(stream_id)
}

/// Escapes special characters in PDF literal string format.
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
