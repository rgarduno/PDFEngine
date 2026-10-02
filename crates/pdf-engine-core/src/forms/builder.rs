//! ISO 32000-1 §12.7 Interactive Form Field Builder & Designer.
//!
//! Provides creation, insertion, updating, and deletion of AcroForm field widgets
//! on arbitrary PDF pages, initializing the catalog `/AcroForm` dictionary if absent.

use crate::cos::object::{ObjectId, PdfArray, PdfDictionary, PdfName, PdfObject, PdfString};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::forms::filler::fill_field_value;
use crate::forms::reader::extract_document_forms;
use crate::forms::types::{FormField, FormFieldType};
use crate::layout::geometry::Rect;

/// Options for creating and placing a new AcroForm interactive form field widget.
#[derive(Debug, Clone)]
pub struct FormFieldCreateOptions {
    /// Fully-qualified field name (e.g. `ClientName`, `AgreeTerms`). Must be unique.
    pub name: String,
    /// Type of form field (Text, Checkbox, RadioButton, PushButton, Choice, Signature).
    pub field_type: FormFieldType,
    /// Bounding rectangle in PDF user space coordinates on the target page.
    pub rect: Rect,
    /// Initial value of the field.
    pub value: Option<String>,
    /// Default field value (`/DV`), if any.
    pub default_value: Option<String>,
    /// User-friendly label or tooltip (`/TU`).
    pub alt_name: Option<String>,
    /// Options list for Choice (dropdown / listbox) fields.
    pub options: Option<Vec<String>>,
    /// Whether the field is read-only.
    pub is_read_only: bool,
    /// Whether the field is required for submission.
    pub is_required: bool,
    /// Whether a text field allows multiline text.
    pub is_multiline: bool,
    /// Maximum character length for text fields (`/MaxLen`).
    pub max_length: Option<usize>,
    /// Font size in points for text/choice fields (defaults to 12.0 pt).
    pub font_size: Option<f64>,
}

impl Default for FormFieldCreateOptions {
    fn default() -> Self {
        Self {
            name: "NewField".to_string(),
            field_type: FormFieldType::Text,
            rect: Rect::new(72.0, 700.0, 250.0, 724.0),
            value: None,
            default_value: None,
            alt_name: None,
            options: None,
            is_read_only: false,
            is_required: false,
            is_multiline: false,
            max_length: None,
            font_size: Some(12.0),
        }
    }
}

/// Options for updating an existing form field's layout or properties.
#[derive(Debug, Clone, Default)]
pub struct FormFieldUpdateOptions {
    /// New bounding box, if updating geometry.
    pub rect: Option<Rect>,
    /// New user-facing tooltip or label.
    pub alt_name: Option<String>,
    /// New read-only status.
    pub is_read_only: Option<bool>,
    /// New required status.
    pub is_required: Option<bool>,
    /// New multiline status (text fields).
    pub is_multiline: Option<bool>,
}

/// Retrieves the existing `/AcroForm` dictionary or creates and links a new one in `/Root`.
pub fn get_or_create_acroform(doc: &mut PdfDocument) -> PdfResult<ObjectId> {
    let catalog = doc.catalog()?;
    if let Some(acro_ref) = catalog.get("AcroForm") {
        if let Some(id) = acro_ref.as_reference() {
            if let Ok(PdfObject::Dictionary(mut d)) = doc.get_object(id) {
                let mut modified = false;
                if !d.contains_key("Fields") {
                    d.insert("Fields", PdfObject::Array(PdfArray::new()));
                    modified = true;
                }
                if !d.contains_key("NeedAppearances") {
                    d.insert("NeedAppearances", PdfObject::Boolean(true));
                    modified = true;
                }
                if modified {
                    doc.set_object(id, PdfObject::Dictionary(d));
                }
                return Ok(id);
            }
        } else if let Some(PdfObject::Dictionary(d)) = catalog.get("AcroForm") {
            let new_id = doc.alloc_object_id();
            let mut d_clone = d.clone();
            if !d_clone.contains_key("Fields") {
                d_clone.insert("Fields", PdfObject::Array(PdfArray::new()));
            }
            d_clone.insert("NeedAppearances", PdfObject::Boolean(true));
            doc.set_object(new_id, PdfObject::Dictionary(d_clone));

            let root_id = doc.catalog_id().ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "Document trailer missing mandatory /Root reference".to_string(),
            })?;
            let mut root_dict = match doc.get_object(root_id)? {
                PdfObject::Dictionary(dict) => dict,
                _ => return Err(PdfError::OperationError("Catalog root is not a dictionary".into())),
            };
            root_dict.insert("AcroForm", PdfObject::Reference(new_id));
            doc.set_object(root_id, PdfObject::Dictionary(root_dict));
            return Ok(new_id);
        }
    }

    // Allocate new AcroForm object
    let acroform_id = doc.alloc_object_id();
    let mut acro_dict = PdfDictionary::new();
    acro_dict.insert("Type", PdfObject::Name(PdfName::new("AcroForm")));
    acro_dict.insert("Fields", PdfObject::Array(PdfArray::new()));
    acro_dict.insert("NeedAppearances", PdfObject::Boolean(true));
    acro_dict.insert(
        "DA",
        PdfObject::String(PdfString::literal(b"/Helv 12 Tf 0 0 0 rg".to_vec())),
    );

    // Standard font resources (/DR)
    let mut res_dict = PdfDictionary::new();
    let mut font_dict = PdfDictionary::new();
    let mut helv_dict = PdfDictionary::new();
    helv_dict.insert("Type", PdfObject::Name(PdfName::new("Font")));
    helv_dict.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
    helv_dict.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
    font_dict.insert("Helv", PdfObject::Dictionary(helv_dict));

    let mut zadb_dict = PdfDictionary::new();
    zadb_dict.insert("Type", PdfObject::Name(PdfName::new("Font")));
    zadb_dict.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
    zadb_dict.insert("BaseFont", PdfObject::Name(PdfName::new("ZapfDingbats")));
    font_dict.insert("ZaDb", PdfObject::Dictionary(zadb_dict));

    res_dict.insert("Font", PdfObject::Dictionary(font_dict));
    acro_dict.insert("DR", PdfObject::Dictionary(res_dict));

    doc.set_object(acroform_id, PdfObject::Dictionary(acro_dict));

    // Link into catalog root
    let root_id = doc.catalog_id().ok_or_else(|| PdfError::InvalidXRef {
        offset: 0,
        message: "Document trailer missing mandatory /Root reference".to_string(),
    })?;
    let mut root_dict = match doc.get_object(root_id)? {
        PdfObject::Dictionary(dict) => dict,
        _ => return Err(PdfError::OperationError("Catalog root is not a dictionary".into())),
    };
    root_dict.insert("AcroForm", PdfObject::Reference(acroform_id));
    doc.set_object(root_id, PdfObject::Dictionary(root_dict));

    Ok(acroform_id)
}

/// Creates a new interactive AcroForm field widget on the specified page.
pub fn create_form_field(
    doc: &mut PdfDocument,
    page_number: usize,
    options: &FormFieldCreateOptions,
) -> PdfResult<FormField> {
    let clean_name = options.name.trim();
    if clean_name.is_empty() {
        return Err(PdfError::OperationError("Field name cannot be empty".into()));
    }

    let page_ids = doc.get_pages()?;
    if page_number == 0 || page_number > page_ids.len() {
        return Err(PdfError::InvalidPageNumber {
            page: page_number,
            total: page_ids.len(),
        });
    }
    let page_id = page_ids[page_number - 1];

    // Check for duplicate field name
    let existing_fields = extract_document_forms(doc)?;
    if existing_fields.iter().any(|f| f.name == clean_name) {
        return Err(PdfError::OperationError(format!(
            "A form field with name '{}' already exists in the document",
            clean_name
        )));
    }

    let acroform_id = get_or_create_acroform(doc)?;
    let field_id = doc.alloc_object_id();

    let mut dict = PdfDictionary::new();
    dict.insert("Type", PdfObject::Name(PdfName::new("Annot")));
    dict.insert("Subtype", PdfObject::Name(PdfName::new("Widget")));
    dict.insert("P", PdfObject::Reference(page_id));
    dict.insert("F", PdfObject::Integer(4)); // Print flag

    // Rect [min_x, min_y, max_x, max_y]
    let mut rect_arr = PdfArray::new();
    rect_arr.push(PdfObject::Real(options.rect.min_x));
    rect_arr.push(PdfObject::Real(options.rect.min_y));
    rect_arr.push(PdfObject::Real(options.rect.max_x));
    rect_arr.push(PdfObject::Real(options.rect.max_y));
    dict.insert("Rect", PdfObject::Array(rect_arr));

    // Field Name (/T)
    dict.insert(
        "T",
        PdfObject::String(PdfString::literal(clean_name.as_bytes().to_vec())),
    );

    if let Some(ref alt) = options.alt_name {
        dict.insert(
            "TU",
            PdfObject::String(PdfString::literal(alt.as_bytes().to_vec())),
        );
    }

    // Flags (/Ff)
    let mut flags: u32 = 0;
    if options.is_read_only {
        flags |= 1; // Bit 1: ReadOnly
    }
    if options.is_required {
        flags |= 2; // Bit 2: Required
    }

    let font_size = options.font_size.unwrap_or(12.0).clamp(6.0, 48.0);
    let default_val = options.value.as_deref().unwrap_or("");

    match options.field_type {
        FormFieldType::Text => {
            dict.insert("FT", PdfObject::Name(PdfName::new("Tx")));
            if options.is_multiline {
                flags |= 4096; // Bit 13: Multiline
            }
            if let Some(max_len) = options.max_length {
                dict.insert("MaxLen", PdfObject::Integer(max_len as i64));
            }
            dict.insert(
                "DA",
                PdfObject::String(PdfString::literal(
                    format!("/Helv {:.1} Tf 0 0 0 rg", font_size).into_bytes(),
                )),
            );
            dict.insert(
                "V",
                PdfObject::String(PdfString::literal(default_val.as_bytes().to_vec())),
            );
            if let Some(ref dv) = options.default_value {
                dict.insert(
                    "DV",
                    PdfObject::String(PdfString::literal(dv.as_bytes().to_vec())),
                );
            }
        }
        FormFieldType::Checkbox => {
            dict.insert("FT", PdfObject::Name(PdfName::new("Btn")));
            let is_checked = {
                let v = default_val.trim().to_lowercase();
                v == "yes" || v == "true" || v == "1" || v == "on"
            };
            let state = if is_checked { "Yes" } else { "Off" };
            dict.insert("V", PdfObject::Name(PdfName::new(state)));
            dict.insert("AS", PdfObject::Name(PdfName::new(state)));
        }
        FormFieldType::RadioButton => {
            dict.insert("FT", PdfObject::Name(PdfName::new("Btn")));
            flags |= 32768; // Bit 16: Radio
            let state = if default_val.is_empty() {
                "Off"
            } else {
                default_val
            };
            dict.insert("V", PdfObject::Name(PdfName::new(state)));
            dict.insert("AS", PdfObject::Name(PdfName::new(state)));
        }
        FormFieldType::PushButton => {
            dict.insert("FT", PdfObject::Name(PdfName::new("Btn")));
            flags |= 65536; // Bit 17: Pushbutton
            dict.insert(
                "V",
                PdfObject::String(PdfString::literal(default_val.as_bytes().to_vec())),
            );
        }
        FormFieldType::Choice => {
            dict.insert("FT", PdfObject::Name(PdfName::new("Ch")));
            flags |= 131072; // Bit 18: Combo
            let mut opt_arr = PdfArray::new();
            let mut selected = default_val.to_string();
            if let Some(ref opts) = options.options {
                for opt in opts {
                    opt_arr.push(PdfObject::String(PdfString::literal(opt.as_bytes().to_vec())));
                }
                if selected.is_empty() && !opts.is_empty() {
                    selected = opts[0].clone();
                }
            }
            dict.insert("Opt", PdfObject::Array(opt_arr));
            dict.insert(
                "V",
                PdfObject::String(PdfString::literal(selected.as_bytes().to_vec())),
            );
            dict.insert(
                "DA",
                PdfObject::String(PdfString::literal(
                    format!("/Helv {:.1} Tf 0 0 0 rg", font_size).into_bytes(),
                )),
            );
        }
        FormFieldType::Signature => {
            dict.insert("FT", PdfObject::Name(PdfName::new("Sig")));
        }
    }

    if flags != 0 {
        dict.insert("Ff", PdfObject::Integer(flags as i64));
    }

    // Save field object
    doc.set_object(field_id, PdfObject::Dictionary(dict));

    // Append to /AcroForm /Fields
    let mut acro_dict = match doc.get_object(acroform_id)? {
        PdfObject::Dictionary(d) => d,
        _ => return Err(PdfError::OperationError("AcroForm is not a dictionary".into())),
    };
    let mut fields_arr = match acro_dict.remove("Fields") {
        Some(PdfObject::Array(arr)) => arr,
        _ => PdfArray::new(),
    };
    fields_arr.push(PdfObject::Reference(field_id));
    acro_dict.insert("Fields", PdfObject::Array(fields_arr));
    doc.set_object(acroform_id, PdfObject::Dictionary(acro_dict));

    // Append to Page /Annots
    let mut page_dict = match doc.get_object(page_id)? {
        PdfObject::Dictionary(d) => d,
        _ => return Err(PdfError::OperationError("Page is not a dictionary".into())),
    };
    let mut annots_arr = match page_dict.remove("Annots") {
        Some(PdfObject::Array(arr)) => arr,
        Some(PdfObject::Reference(r)) => match doc.get_object(r)? {
            PdfObject::Array(arr) => arr,
            _ => PdfArray::new(),
        },
        _ => PdfArray::new(),
    };
    annots_arr.push(PdfObject::Reference(field_id));
    page_dict.insert("Annots", PdfObject::Array(annots_arr));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    // Synthesize appearance if field has value and is not signature
    if options.field_type != FormFieldType::Signature && !default_val.is_empty() {
        let _ = fill_field_value(doc, clean_name, default_val);
    }

    Ok(FormField {
        id: field_id,
        name: clean_name.to_string(),
        alt_name: options.alt_name.clone(),
        field_type: options.field_type,
        value: default_val.to_string(),
        default_value: options.default_value.clone(),
        rect: options.rect,
        page_number,
        page_id,
        options: options.options.clone().unwrap_or_default(),
        flags,
        is_read_only: options.is_read_only,
        is_required: options.is_required,
        is_multiline: options.is_multiline,
        max_length: options.max_length,
    })
}

/// Deletes an existing interactive form field from the document and all associated page annotations.
pub fn delete_form_field(doc: &mut PdfDocument, name: &str) -> PdfResult<bool> {
    let clean_name = name.trim();
    let fields = extract_document_forms(doc)?;
    let target = match fields
        .iter()
        .find(|f| f.name == clean_name || f.id.number.to_string() == clean_name)
    {
        Some(f) => f.clone(),
        None => return Ok(false),
    };

    // 1. Remove from /AcroForm /Fields
    let catalog = doc.catalog()?;
    if let Some(acro_obj) = catalog.get("AcroForm") {
        let acro_id = acro_obj.as_reference();
        if let Some(aid) = acro_id {
            if let Ok(PdfObject::Dictionary(mut adict)) = doc.get_object(aid) {
                if let Some(PdfObject::Array(mut arr)) = adict.remove("Fields") {
                    arr.retain(|item| item.as_reference() != Some(target.id));
                    adict.insert("Fields", PdfObject::Array(arr));
                    doc.set_object(aid, PdfObject::Dictionary(adict));
                }
            }
        }
    }

    // 2. Remove from Page /Annots
    if let Ok(page_obj) = doc.get_object(target.page_id) {
        if let Some(page_dict) = page_obj.as_dict() {
            let mut pd = page_dict.clone();
            let mut modified = false;
            if let Some(PdfObject::Array(mut annots)) = pd.remove("Annots") {
                annots.retain(|item| item.as_reference() != Some(target.id));
                pd.insert("Annots", PdfObject::Array(annots));
                modified = true;
            } else if let Some(PdfObject::Reference(r)) = pd.get("Annots") {
                if let Ok(PdfObject::Array(mut annots)) = doc.get_object(*r) {
                    annots.retain(|item| item.as_reference() != Some(target.id));
                    doc.set_object(*r, PdfObject::Array(annots));
                }
            }
            if modified {
                doc.set_object(target.page_id, PdfObject::Dictionary(pd));
            }
        }
    }

    // 3. Remove field object and its appearance streams
    if let Ok(PdfObject::Dictionary(fdict)) = doc.get_object(target.id) {
        if let Some(PdfObject::Dictionary(ap_dict)) = fdict.get("AP") {
            for (_, val) in ap_dict.iter() {
                if let Some(stream_id) = val.as_reference() {
                    doc.objects.remove(&stream_id);
                }
            }
        }
    }
    doc.objects.remove(&target.id);

    Ok(true)
}

/// Updates the bounding rectangle or properties of an existing form field.
pub fn update_form_field(
    doc: &mut PdfDocument,
    name: &str,
    options: &FormFieldUpdateOptions,
) -> PdfResult<Option<FormField>> {
    let clean_name = name.trim();
    let fields = extract_document_forms(doc)?;
    let target = match fields
        .iter()
        .find(|f| f.name == clean_name || f.id.number.to_string() == clean_name)
    {
        Some(f) => f.clone(),
        None => return Ok(None),
    };

    let mut dict = match doc.get_object(target.id)? {
        PdfObject::Dictionary(d) => d,
        _ => {
            return Err(PdfError::ObjectNotFound {
                id: target.id.number,
                gen: target.id.generation,
            })
        }
    };

    if let Some(new_rect) = options.rect {
        let mut rect_arr = PdfArray::new();
        rect_arr.push(PdfObject::Real(new_rect.min_x));
        rect_arr.push(PdfObject::Real(new_rect.min_y));
        rect_arr.push(PdfObject::Real(new_rect.max_x));
        rect_arr.push(PdfObject::Real(new_rect.max_y));
        dict.insert("Rect", PdfObject::Array(rect_arr));
    }

    if let Some(ref alt) = options.alt_name {
        dict.insert(
            "TU",
            PdfObject::String(PdfString::literal(alt.as_bytes().to_vec())),
        );
    }

    let mut flags = dict.get("Ff").and_then(|f| f.as_i64()).unwrap_or(0) as u32;
    if let Some(ro) = options.is_read_only {
        if ro {
            flags |= 1;
        } else {
            flags &= !1;
        }
    }
    if let Some(req) = options.is_required {
        if req {
            flags |= 2;
        } else {
            flags &= !2;
        }
    }
    if let Some(multi) = options.is_multiline {
        if multi {
            flags |= 4096;
        } else {
            flags &= !4096;
        }
    }
    dict.insert("Ff", PdfObject::Integer(flags as i64));

    doc.set_object(target.id, PdfObject::Dictionary(dict));

    // Re-extract updated field
    let updated_fields = extract_document_forms(doc)?;
    Ok(updated_fields.into_iter().find(|f| f.name == target.name))
}
