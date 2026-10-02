//! Reader and parser for ISO 32000-1 §12.7 AcroForm dictionaries and field hierarchies.

use std::collections::HashMap;
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject};
use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::forms::types::{FormField, FormFieldType};
use crate::layout::geometry::Rect;

/// Extracts all interactive form fields from the document's `/AcroForm` catalog entry.
pub fn extract_document_forms(doc: &mut PdfDocument) -> PdfResult<Vec<FormField>> {
    let catalog = doc.catalog()?;
    let acroform_obj = match catalog.get("AcroForm") {
        Some(PdfObject::Dictionary(d)) => Some(d.clone()),
        Some(PdfObject::Reference(r)) => match doc.get_object(*r)? {
            PdfObject::Dictionary(d) => Some(d),
            _ => None,
        },
        _ => None,
    };

    let acroform = match acroform_obj {
        Some(af) => af,
        None => return Ok(Vec::new()),
    };

    let fields_arr = match acroform.get("Fields") {
        Some(PdfObject::Array(arr)) => arr.clone(),
        Some(PdfObject::Reference(r)) => match doc.get_object(*r)? {
            PdfObject::Array(arr) => arr,
            _ => return Ok(Vec::new()),
        },
        _ => return Ok(Vec::new()),
    };

    // Pre-map pages to their 1-based index and map page annot references
    let page_ids = doc.get_pages()?;
    let mut page_id_to_num: HashMap<ObjectId, usize> = HashMap::new();
    let mut annot_to_page: HashMap<ObjectId, (usize, ObjectId)> = HashMap::new();

    for (idx, &page_id) in page_ids.iter().enumerate() {
        let page_num = idx + 1;
        page_id_to_num.insert(page_id, page_num);

        if let Ok(page_obj) = doc.get_object(page_id) {
            if let Some(page_dict) = page_obj.as_dict() {
                if let Some(annots) = page_dict.get("Annots").and_then(|a| a.as_array()) {
                    for annot_ref in annots.iter().filter_map(|a| a.as_reference()) {
                        annot_to_page.insert(annot_ref, (page_num, page_id));
                    }
                }
            }
        }
    }

    let default_page = page_ids.first().copied().unwrap_or(ObjectId::new(1));
    let mut extracted_fields = Vec::new();

    for field_item in fields_arr.iter() {
        if let Some(field_ref) = field_item.as_reference() {
            extract_field_recursive(
                doc,
                field_ref,
                "",
                None,
                &page_id_to_num,
                &annot_to_page,
                default_page,
                &mut extracted_fields,
                0,
            )?;
        }
    }

    Ok(extracted_fields)
}

/// Recursively traverses a field hierarchy resolving inherited attributes.
fn extract_field_recursive(
    doc: &mut PdfDocument,
    field_id: ObjectId,
    parent_name: &str,
    parent_ft: Option<FormFieldType>,
    page_id_to_num: &HashMap<ObjectId, usize>,
    annot_to_page: &HashMap<ObjectId, (usize, ObjectId)>,
    default_page: ObjectId,
    results: &mut Vec<FormField>,
    depth: usize,
) -> PdfResult<()> {
    if depth > 32 {
        return Ok(());
    }

    let field_obj = doc.get_object(field_id)?;
    let field_dict = match field_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(()),
    };

    // Determine field partial name (/T) and fully qualified name
    let partial_name = field_dict
        .get("T")
        .and_then(|t| t.as_string_bytes())
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();

    let full_name = if parent_name.is_empty() {
        partial_name.clone()
    } else if partial_name.is_empty() {
        parent_name.to_string()
    } else {
        format!("{}.{}", parent_name, partial_name)
    };

    // Flags (/Ff)
    let flags = field_dict.get("Ff").and_then(|f| f.as_i64()).unwrap_or(0) as u32;

    // Determine Field Type (/FT) with inheritance
    let field_type = if let Some(ft_name) = field_dict.get("FT").and_then(|ft| ft.as_name()) {
        match ft_name {
            "Tx" => Some(FormFieldType::Text),
            "Btn" => {
                if (flags & 0x10000) != 0 {
                    Some(FormFieldType::PushButton)
                } else if (flags & 0x8000) != 0 {
                    Some(FormFieldType::RadioButton)
                } else {
                    Some(FormFieldType::Checkbox)
                }
            }
            "Ch" => Some(FormFieldType::Choice),
            "Sig" => Some(FormFieldType::Signature),
            _ => None,
        }
    } else {
        parent_ft
    };

    // Check if this field has Kids
    let kids = field_dict.get("Kids").and_then(|k| k.as_array()).map(|arr| arr.to_vec());

    // A field is non-terminal if it has Kids that are themselves fields (not just widget annotations)
    // In PDF, if a field has kids with /T, they are child fields. If kids have no /T, they are multiple widget instances of this field.
    let mut has_child_fields = false;
    if let Some(ref kids_arr) = kids {
        for kid_item in kids_arr.iter() {
            if let Some(kid_ref) = kid_item.as_reference() {
                if let Ok(kid_obj) = doc.get_object(kid_ref) {
                    if let Some(kid_dict) = kid_obj.as_dict() {
                        if kid_dict.contains_key("T") {
                            has_child_fields = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    if has_child_fields {
        if let Some(kids_arr) = kids {
            for kid_item in kids_arr.iter() {
                if let Some(kid_ref) = kid_item.as_reference() {
                    extract_field_recursive(
                        doc,
                        kid_ref,
                        &full_name,
                        field_type,
                        page_id_to_num,
                        annot_to_page,
                        default_page,
                        results,
                        depth + 1,
                    )?;
                }
            }
        }
        return Ok(());
    }

    // Terminal field: extract values and geometry
    let effective_type = field_type.unwrap_or(FormFieldType::Text);

    // Value (/V)
    let value = parse_field_value(&field_dict);
    let default_val = field_dict.get("DV").map(|dv| match dv {
        PdfObject::String(s) => s.to_string_lossy(),
        PdfObject::Name(n) => n.0.clone(),
        _ => String::new(),
    });

    let alt_name = field_dict
        .get("TU")
        .and_then(|tu| tu.as_string_bytes())
        .map(|b| String::from_utf8_lossy(b).into_owned());

    // Options (/Opt) for choice fields
    let mut options = Vec::new();
    if let Some(opt_arr) = field_dict.get("Opt").and_then(|o| o.as_array()) {
        for opt_item in opt_arr.iter() {
            match opt_item {
                PdfObject::String(s) => options.push(s.to_string_lossy()),
                PdfObject::Array(pair) => {
                    // [export_val, display_val]
                    if let Some(display) = pair.get(1).and_then(|p| p.as_string_bytes()) {
                        options.push(String::from_utf8_lossy(display).into_owned());
                    } else if let Some(export) = pair.get(0).and_then(|p| p.as_string_bytes()) {
                        options.push(String::from_utf8_lossy(export).into_owned());
                    }
                }
                _ => {}
            }
        }
    }

    // Geometry (/Rect) and Page association (/P or annot_to_page)
    let (rect, page_num, page_id) = resolve_field_geometry_and_page(
        &field_dict,
        field_id,
        kids.as_deref(),
        page_id_to_num,
        annot_to_page,
        default_page,
        doc,
    );

    let is_read_only = (flags & 0x1) != 0;
    let is_required = (flags & 0x2) != 0;
    let is_multiline = (flags & 0x1000) != 0;
    let max_length = field_dict
        .get("MaxLen")
        .and_then(|m| m.as_i64())
        .map(|m| m as usize);

    results.push(FormField {
        id: field_id,
        name: if full_name.is_empty() {
            format!("Field_{}", field_id.number)
        } else {
            full_name
        },
        alt_name,
        field_type: effective_type,
        value,
        default_value: default_val,
        rect,
        page_number: page_num,
        page_id,
        options,
        flags,
        is_read_only,
        is_required,
        is_multiline,
        max_length,
    });

    Ok(())
}

/// Parses the `/V` value from a field dictionary into a clean string representation.
fn parse_field_value(dict: &PdfDictionary) -> String {
    match dict.get("V") {
        Some(PdfObject::String(s)) => s.to_string_lossy(),
        Some(PdfObject::Name(n)) => n.0.clone(),
        Some(PdfObject::Integer(i)) => i.to_string(),
        Some(PdfObject::Real(r)) => format!("{:.2}", r),
        Some(PdfObject::Boolean(b)) => {
            if *b {
                "Yes".to_string()
            } else {
                "Off".to_string()
            }
        }
        Some(PdfObject::Array(arr)) => {
            // Choice field multi-selection: join with commas
            let vals: Vec<String> = arr
                .iter()
                .filter_map(|item| match item {
                    PdfObject::String(s) => Some(s.to_string_lossy()),
                    PdfObject::Name(n) => Some(n.0.clone()),
                    _ => None,
                })
                .collect();
            vals.join(", ")
        }
        _ => String::new(),
    }
}

/// Resolves field rectangle and page mapping from the dictionary or its widget kids.
fn resolve_field_geometry_and_page(
    dict: &PdfDictionary,
    field_id: ObjectId,
    kids: Option<&[PdfObject]>,
    page_id_to_num: &HashMap<ObjectId, usize>,
    annot_to_page: &HashMap<ObjectId, (usize, ObjectId)>,
    default_page: ObjectId,
    doc: &mut PdfDocument,
) -> (Rect, usize, ObjectId) {
    // 1. Try finding /Rect directly on field
    if let Some(rect) = parse_rect(dict.get("Rect")) {
        let (page_num, page_id) = resolve_page_for_dict(dict, field_id, page_id_to_num, annot_to_page, default_page);
        return (rect, page_num, page_id);
    }

    // 2. Try looking into first kid widget
    if let Some(kids_arr) = kids {
        for kid in kids_arr {
            if let Some(kid_ref) = kid.as_reference() {
                if let Ok(kid_obj) = doc.get_object(kid_ref) {
                    if let Some(kid_dict) = kid_obj.as_dict() {
                        if let Some(rect) = parse_rect(kid_dict.get("Rect")) {
                            let (page_num, page_id) = resolve_page_for_dict(
                                kid_dict,
                                kid_ref,
                                page_id_to_num,
                                annot_to_page,
                                default_page,
                            );
                            return (rect, page_num, page_id);
                        }
                    }
                }
            }
        }
    }

    // Default fallback rect
    (Rect::new(72.0, 700.0, 200.0, 720.0), 1, default_page)
}

/// Resolves which page an object or dictionary belongs to.
fn resolve_page_for_dict(
    dict: &PdfDictionary,
    obj_id: ObjectId,
    page_id_to_num: &HashMap<ObjectId, usize>,
    annot_to_page: &HashMap<ObjectId, (usize, ObjectId)>,
    default_page: ObjectId,
) -> (usize, ObjectId) {
    // Check direct /P key
    if let Some(p_ref) = dict.get("P").and_then(|p| p.as_reference()) {
        if let Some(&page_num) = page_id_to_num.get(&p_ref) {
            return (page_num, p_ref);
        }
    }

    // Check annot_to_page mapping
    if let Some(&(page_num, page_id)) = annot_to_page.get(&obj_id) {
        return (page_num, page_id);
    }

    (1, default_page)
}

/// Parses a 4-number array into a geometry `Rect`.
fn parse_rect(obj: Option<&PdfObject>) -> Option<Rect> {
    let arr = match obj {
        Some(PdfObject::Array(a)) if a.len() >= 4 => a,
        _ => return None,
    };

    let p0 = arr[0].as_f64()?;
    let p1 = arr[1].as_f64()?;
    let p2 = arr[2].as_f64()?;
    let p3 = arr[3].as_f64()?;

    let min_x = p0.min(p2);
    let min_y = p1.min(p3);
    let max_x = p0.max(p2);
    let max_y = p1.max(p3);

    Some(Rect::new(min_x, min_y, max_x, max_y))
}
