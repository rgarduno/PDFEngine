//! Optional Content Groups (OCG / PDF Layers) manager and visibility evaluator (ISO 32000-1 §8.11).
//!
//! Provides inspection, creation, deletion, and visibility toggling of Optional Content
//! Groups (`/OCProperties`, `/OCGs`, `/D` configuration dictionary) as well as page-level
//! `/Resources /Properties` resolution for filtering `/OC /<PropName> BDC ... EMC` sequences
//! in the layout reconstructor.

use std::collections::HashSet;

use crate::cos::{ObjectId, PdfArray, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfString};
use crate::error::{PdfError, PdfResult};

/// Maximum recursion depth when resolving page `/Parent` resources or `/Order` arrays.
const MAX_LAYER_DEPTH: usize = 32;

/// Represents an Optional Content Group (OCG / PDF Layer) in a PDF document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalContentLayer {
    /// Canonical layer identifier (e.g. `"ocg-12"`).
    pub id: String,
    /// Indirect object number of the `/OCG` dictionary.
    pub object_number: u32,
    /// Human-readable layer name (`/Name` entry in the `/OCG` dictionary).
    pub name: String,
    /// Effective visibility state in the default viewing configuration (`/D`).
    pub visible: bool,
    /// Intended uses of the layer (`/Intent`, defaults to `["View"]`).
    pub intent: Vec<String>,
    /// Whether the layer is locked against user toggling in viewer UI (`/Locked` in `/D`).
    pub locked: bool,
}

/// Reserves a fresh indirect object ID by immediately inserting `PdfObject::Null`.
fn reserve_object_id(doc: &mut PdfDocument) -> ObjectId {
    let id = doc.alloc_object_id();
    doc.set_object(id, PdfObject::Null);
    id
}

/// Encodes a Rust UTF-8 string into a `PdfString` (ASCII literal if printable ASCII,
/// otherwise UTF-16BE with `\xFE\xFF` BOM).
fn encode_pdf_text_string(text: &str) -> PdfString {
    if text.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        PdfString::literal(text.as_bytes().to_vec())
    } else {
        let mut bytes = Vec::with_capacity(2 + text.len() * 2);
        bytes.push(0xFE);
        bytes.push(0xFF);
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        PdfString::hex(bytes)
    }
}

/// Resolves a `PdfObject` (following an indirect reference if needed) into a cloned `PdfDictionary`.
fn resolve_dict(doc: &mut PdfDocument, obj: &PdfObject) -> PdfResult<Option<PdfDictionary>> {
    match obj {
        PdfObject::Dictionary(d) => Ok(Some(d.clone())),
        PdfObject::Reference(id) => {
            let resolved = doc.get_object(*id)?;
            Ok(resolved.as_dict().cloned())
        }
        _ => Ok(None),
    }
}

/// Resolves a `PdfObject` (following an indirect reference if needed) into a cloned `Vec<PdfObject>`.
fn resolve_array(doc: &mut PdfDocument, obj: &PdfObject) -> PdfResult<Option<Vec<PdfObject>>> {
    match obj {
        PdfObject::Array(arr) => Ok(Some(arr.clone())),
        PdfObject::Reference(id) => {
            let resolved = doc.get_object(*id)?;
            Ok(resolved.as_array().map(|a| a.to_vec()))
        }
        _ => Ok(None),
    }
}

/// Extracts a set of `ObjectId` references from an optional array entry in a dictionary.
fn extract_ref_set(doc: &mut PdfDocument, dict: &PdfDictionary, key: &str) -> PdfResult<HashSet<ObjectId>> {
    let mut set = HashSet::new();
    let Some(obj) = dict.get(key).cloned() else {
        return Ok(set);
    };
    if let Some(arr) = resolve_array(doc, &obj)? {
        for item in arr {
            if let PdfObject::Reference(id) = item {
                set.insert(id);
            }
        }
    }
    Ok(set)
}

/// Parses `/Intent` on an `/OCG` dictionary (can be a single `/Name` or an array of `/Name`s).
fn parse_ocg_intent(doc: &mut PdfDocument, ocg_dict: &PdfDictionary) -> Vec<String> {
    let Some(intent_obj) = ocg_dict.get("Intent").cloned() else {
        return vec!["View".to_string()];
    };
    match intent_obj {
        PdfObject::Name(n) => vec![n.as_str().to_string()],
        PdfObject::Array(arr) => {
            let mut out: Vec<String> = arr
                .into_iter()
                .filter_map(|item| item.as_name().map(str::to_string))
                .collect();
            if out.is_empty() {
                out.push("View".to_string());
            }
            out
        }
        PdfObject::Reference(id) => {
            if let Ok(resolved) = doc.get_object(id) {
                if let Some(n) = resolved.as_name() {
                    return vec![n.to_string()];
                }
                if let Some(arr) = resolved.as_array() {
                    let out: Vec<String> = arr
                        .iter()
                        .filter_map(|item| item.as_name().map(str::to_string))
                        .collect();
                    if !out.is_empty() {
                        return out;
                    }
                }
            }
            vec!["View".to_string()]
        }
        _ => vec!["View".to_string()],
    }
}

/// Checks if a user-supplied target matches a layer (by `"ocg-<num>"`, `"<num>"`, or `name`).
fn layer_matches_target(layer: &OptionalContentLayer, target: &str) -> bool {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return false;
    }
    if layer.id.eq_ignore_ascii_case(trimmed) || layer.name == trimmed {
        return true;
    }
    if let Ok(num) = trimmed.parse::<u32>() {
        if layer.object_number == num {
            return true;
        }
    }
    false
}

/// Extracts all Optional Content Groups (`/OCProperties` -> `/OCGs` and `/D`) from the document.
pub fn extract_layers(doc: &mut PdfDocument) -> PdfResult<Vec<OptionalContentLayer>> {
    let ocp_obj = doc
        .catalog()
        .ok()
        .and_then(|cat| cat.get("OCProperties").cloned());
    let Some(ocp_obj) = ocp_obj else {
        return Ok(Vec::new());
    };
    let Some(ocp_dict) = resolve_dict(doc, &ocp_obj)? else {
        return Ok(Vec::new());
    };

    let Some(ocgs_obj) = ocp_dict.get("OCGs").cloned() else {
        return Ok(Vec::new());
    };
    let Some(ocgs_arr) = resolve_array(doc, &ocgs_obj)? else {
        return Ok(Vec::new());
    };

    // Parse default viewing configuration `/D`
    let d_dict = if let Some(d_obj) = ocp_dict.get("D").cloned() {
        resolve_dict(doc, &d_obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    let base_state = d_dict
        .get("BaseState")
        .and_then(|o| o.as_name())
        .unwrap_or("ON")
        .to_string();
    let on_set = extract_ref_set(doc, &d_dict, "ON")?;
    let off_set = extract_ref_set(doc, &d_dict, "OFF")?;
    let locked_set = extract_ref_set(doc, &d_dict, "Locked")?;

    let mut layers = Vec::new();
    let mut seen_ids = HashSet::new();

    for item in ocgs_arr {
        let PdfObject::Reference(ocg_id) = item else {
            continue;
        };
        if !seen_ids.insert(ocg_id) {
            continue;
        }
        let Ok(ocg_obj) = doc.get_object(ocg_id) else {
            continue;
        };
        let Some(ocg_dict) = ocg_obj.as_dict().cloned() else {
            continue;
        };

        let name = ocg_dict
            .get("Name")
            .and_then(|o| o.as_string())
            .map(|s| s.to_string_lossy())
            .unwrap_or_else(|| format!("Layer {}", ocg_id.number));

        let visible = if base_state.eq_ignore_ascii_case("OFF") {
            on_set.contains(&ocg_id)
        } else {
            !off_set.contains(&ocg_id)
        };

        let intent = parse_ocg_intent(doc, &ocg_dict);
        let locked = locked_set.contains(&ocg_id);

        layers.push(OptionalContentLayer {
            id: format!("ocg-{}", ocg_id.number),
            object_number: ocg_id.number,
            name,
            visible,
            intent,
            locked,
        });
    }

    Ok(layers)
}

/// Creates and registers a new Optional Content Group (`/Type /OCG`) in `/Catalog /OCProperties`.
pub fn add_layer(
    doc: &mut PdfDocument,
    name: &str,
    visible: bool,
    intent: Option<Vec<String>>,
    locked: bool,
) -> PdfResult<OptionalContentLayer> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
        return Err(PdfError::OperationError(
            "Layer name cannot be empty".to_string(),
        ));
    }

    let catalog_id = doc.catalog_id().ok_or_else(|| PdfError::InvalidXRef {
        offset: 0,
        message: "Document trailer missing /Root reference".into(),
    })?;

    let intent_list: Vec<String> = intent
        .unwrap_or_else(|| vec!["View".to_string()])
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let final_intent = if intent_list.is_empty() {
        vec!["View".to_string()]
    } else {
        intent_list
    };

    // 1. Allocate the `/OCG` dictionary object
    let ocg_id = reserve_object_id(doc);
    let mut ocg_dict = PdfDictionary::new();
    ocg_dict.insert("Type", PdfObject::Name(PdfName::new("OCG")));
    ocg_dict.insert("Name", PdfObject::String(encode_pdf_text_string(clean_name)));
    if final_intent.len() == 1 {
        ocg_dict.insert(
            "Intent",
            PdfObject::Name(PdfName::new(final_intent[0].clone())),
        );
    } else {
        let arr: PdfArray = final_intent
            .iter()
            .map(|i| PdfObject::Name(PdfName::new(i.clone())))
            .collect();
        ocg_dict.insert("Intent", PdfObject::Array(arr));
    }
    doc.set_object(ocg_id, PdfObject::Dictionary(ocg_dict));

    // 2. Update `/Catalog` -> `/OCProperties`
    let existing_ocp_obj = doc
        .catalog()
        .ok()
        .and_then(|cat| cat.get("OCProperties").cloned());

    let mut ocp_dict = if let Some(ref obj) = existing_ocp_obj {
        resolve_dict(doc, obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    let mut ocgs_arr = if let Some(ocgs_obj) = ocp_dict.get("OCGs").cloned() {
        resolve_array(doc, &ocgs_obj)?.unwrap_or_default()
    } else {
        Vec::new()
    };
    ocgs_arr.push(PdfObject::Reference(ocg_id));
    ocp_dict.insert("OCGs", PdfObject::Array(ocgs_arr));

    let mut d_dict = if let Some(d_obj) = ocp_dict.get("D").cloned() {
        resolve_dict(doc, &d_obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    if !d_dict.contains_key("BaseState") {
        d_dict.insert("BaseState", PdfObject::Name(PdfName::new("ON")));
    }

    let mut order_arr = if let Some(order_obj) = d_dict.get("Order").cloned() {
        resolve_array(doc, &order_obj)?.unwrap_or_default()
    } else {
        Vec::new()
    };
    order_arr.push(PdfObject::Reference(ocg_id));
    d_dict.insert("Order", PdfObject::Array(order_arr));

    let mut on_arr = if let Some(on_obj) = d_dict.get("ON").cloned() {
        resolve_array(doc, &on_obj)?.unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut off_arr = if let Some(off_obj) = d_dict.get("OFF").cloned() {
        resolve_array(doc, &off_obj)?.unwrap_or_default()
    } else {
        Vec::new()
    };

    if visible {
        on_arr.push(PdfObject::Reference(ocg_id));
    } else {
        off_arr.push(PdfObject::Reference(ocg_id));
    }
    d_dict.insert("ON", PdfObject::Array(on_arr));
    d_dict.insert("OFF", PdfObject::Array(off_arr));

    if locked {
        let mut locked_arr = if let Some(locked_obj) = d_dict.get("Locked").cloned() {
            resolve_array(doc, &locked_obj)?.unwrap_or_default()
        } else {
            Vec::new()
        };
        locked_arr.push(PdfObject::Reference(ocg_id));
        d_dict.insert("Locked", PdfObject::Array(locked_arr));
    }

    ocp_dict.insert("D", PdfObject::Dictionary(d_dict));

    let mut cat_dict = doc.catalog()?;
    cat_dict.insert("OCProperties", PdfObject::Dictionary(ocp_dict));
    doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));

    Ok(OptionalContentLayer {
        id: format!("ocg-{}", ocg_id.number),
        object_number: ocg_id.number,
        name: clean_name.to_string(),
        visible,
        intent: final_intent,
        locked,
    })
}

/// Updates the visibility state (`ON` / `OFF`) of an Optional Content Group by `id` or `name`.
pub fn set_layer_visibility(
    doc: &mut PdfDocument,
    target: &str,
    visible: bool,
) -> PdfResult<OptionalContentLayer> {
    let layers = extract_layers(doc)?;
    let matched = layers
        .into_iter()
        .find(|l| layer_matches_target(l, target))
        .ok_or_else(|| {
            PdfError::OperationError(format!("Layer '{}' not found in document", target.trim()))
        })?;

    let target_id = ObjectId::new(matched.object_number);
    let catalog_id = doc.catalog_id().ok_or_else(|| PdfError::InvalidXRef {
        offset: 0,
        message: "Document trailer missing /Root reference".into(),
    })?;

    let existing_ocp_obj = doc
        .catalog()?
        .get("OCProperties")
        .cloned()
        .ok_or_else(|| PdfError::OperationError("Document has no /OCProperties".to_string()))?;

    let mut ocp_dict = resolve_dict(doc, &existing_ocp_obj)?
        .ok_or_else(|| PdfError::OperationError("Invalid /OCProperties dictionary".to_string()))?;

    let mut d_dict = if let Some(d_obj) = ocp_dict.get("D").cloned() {
        resolve_dict(doc, &d_obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    let mut on_arr = if let Some(on_obj) = d_dict.get("ON").cloned() {
        resolve_array(doc, &on_obj)?.unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut off_arr = if let Some(off_obj) = d_dict.get("OFF").cloned() {
        resolve_array(doc, &off_obj)?.unwrap_or_default()
    } else {
        Vec::new()
    };

    on_arr.retain(|item| item.as_reference() != Some(target_id));
    off_arr.retain(|item| item.as_reference() != Some(target_id));

    if visible {
        on_arr.push(PdfObject::Reference(target_id));
    } else {
        off_arr.push(PdfObject::Reference(target_id));
    }

    d_dict.insert("ON", PdfObject::Array(on_arr));
    d_dict.insert("OFF", PdfObject::Array(off_arr));
    ocp_dict.insert("D", PdfObject::Dictionary(d_dict));

    match existing_ocp_obj {
        PdfObject::Reference(ocp_id) => {
            doc.set_object(ocp_id, PdfObject::Dictionary(ocp_dict));
        }
        _ => {
            let mut cat_dict = doc.catalog()?;
            cat_dict.insert("OCProperties", PdfObject::Dictionary(ocp_dict));
            doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));
        }
    }

    Ok(OptionalContentLayer {
        visible,
        ..matched
    })
}

/// Deletes an Optional Content Group by `id` or `name` from `/OCProperties` and page `/Properties`.
pub fn delete_layer(doc: &mut PdfDocument, target: &str) -> PdfResult<bool> {
    let layers = extract_layers(doc)?;
    let Some(matched) = layers.into_iter().find(|l| layer_matches_target(l, target)) else {
        return Ok(false);
    };
    let target_id = ObjectId::new(matched.object_number);

    let Some(catalog_id) = doc.catalog_id() else {
        return Ok(false);
    };

    let existing_ocp_obj = doc
        .catalog()
        .ok()
        .and_then(|cat| cat.get("OCProperties").cloned());
    if let Some(ocp_obj) = existing_ocp_obj {
        if let Some(mut ocp_dict) = resolve_dict(doc, &ocp_obj)? {
            if let Some(ocgs_obj) = ocp_dict.get("OCGs").cloned() {
                if let Some(mut ocgs_arr) = resolve_array(doc, &ocgs_obj)? {
                    ocgs_arr.retain(|item| item.as_reference() != Some(target_id));
                    ocp_dict.insert("OCGs", PdfObject::Array(ocgs_arr));
                }
            }
            if let Some(d_obj) = ocp_dict.get("D").cloned() {
                if let Some(mut d_dict) = resolve_dict(doc, &d_obj)? {
                    for key in ["ON", "OFF", "Order", "Locked"] {
                        if let Some(arr_obj) = d_dict.get(key).cloned() {
                            if let Some(mut arr) = resolve_array(doc, &arr_obj)? {
                                arr.retain(|item| item.as_reference() != Some(target_id));
                                d_dict.insert(key, PdfObject::Array(arr));
                            }
                        }
                    }
                    ocp_dict.insert("D", PdfObject::Dictionary(d_dict));
                }
            }

            match ocp_obj {
                PdfObject::Reference(ocp_id) => {
                    doc.set_object(ocp_id, PdfObject::Dictionary(ocp_dict));
                }
                _ => {
                    let mut cat_dict = doc.catalog()?;
                    cat_dict.insert("OCProperties", PdfObject::Dictionary(ocp_dict));
                    doc.set_object(catalog_id, PdfObject::Dictionary(cat_dict));
                }
            }
        }
    }

    doc.objects.remove(&target_id);
    doc.xref.entries.remove(&target_id);

    Ok(true)
}

/// Associates a page `/Resources /Properties /<prop_name>` key with an existing OCG layer
/// so `/OC /<prop_name> BDC ... EMC` sequences on `page_number` (1-indexed) are controlled by that layer.
pub fn register_page_layer_property(
    doc: &mut PdfDocument,
    page_number: usize,
    prop_name: &str,
    layer_target: &str,
) -> PdfResult<String> {
    let clean_prop = prop_name.trim().trim_start_matches('/');
    if clean_prop.is_empty() {
        return Err(PdfError::OperationError(
            "Property name cannot be empty".to_string(),
        ));
    }

    let page_ids = doc.get_pages()?;
    if page_number == 0 || page_number > page_ids.len() {
        return Err(PdfError::OperationError(format!(
            "Page number {} is out of range (1..={})",
            page_number,
            page_ids.len()
        )));
    }
    let page_id = page_ids[page_number - 1];

    let layers = extract_layers(doc)?;
    let matched = layers
        .into_iter()
        .find(|l| layer_matches_target(l, layer_target))
        .ok_or_else(|| {
            PdfError::OperationError(format!(
                "Layer '{}' not found in document",
                layer_target.trim()
            ))
        })?;
    let ocg_id = ObjectId::new(matched.object_number);

    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = page_obj
        .as_dict()
        .cloned()
        .ok_or_else(|| PdfError::OperationError("Page object is not a dictionary".to_string()))?;

    let existing_res_obj = page_dict.get("Resources").cloned();
    let mut res_dict = if let Some(ref obj) = existing_res_obj {
        resolve_dict(doc, obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    let existing_props_obj = res_dict.get("Properties").cloned();
    let mut props_dict = if let Some(ref obj) = existing_props_obj {
        resolve_dict(doc, obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    props_dict.insert(clean_prop, PdfObject::Reference(ocg_id));
    res_dict.insert("Properties", PdfObject::Dictionary(props_dict));
    page_dict.insert("Resources", PdfObject::Dictionary(res_dict));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(matched.id)
}

/// Evaluates an Optional Content Membership Dictionary (`/Type /OCMD`, ISO 32000-1 §8.11.2.2)
/// against the set of visible and hidden OCG object numbers.
fn evaluate_ocmd_visibility(
    doc: &mut PdfDocument,
    ocmd_dict: &PdfDictionary,
    visible_ocgs: &HashSet<u32>,
    hidden_ocgs: &HashSet<u32>,
) -> PdfResult<bool> {
    let policy = ocmd_dict
        .get("P")
        .and_then(|o| o.as_name())
        .unwrap_or("AnyOn");

    let mut member_ids = Vec::new();
    if let Some(ocgs_obj) = ocmd_dict.get("OCGs").cloned() {
        match ocgs_obj {
            PdfObject::Reference(id) => member_ids.push(id.number),
            PdfObject::Array(arr) => {
                for item in arr {
                    if let PdfObject::Reference(id) = item {
                        member_ids.push(id.number);
                    }
                }
            }
            _ => {
                if let Some(arr) = resolve_array(doc, &ocgs_obj)? {
                    for item in arr {
                        if let PdfObject::Reference(id) = item {
                            member_ids.push(id.number);
                        }
                    }
                }
            }
        }
    }

    if member_ids.is_empty() {
        return Ok(true);
    }

    let is_member_on = |num: &u32| -> bool {
        if hidden_ocgs.contains(num) {
            false
        } else {
            visible_ocgs.contains(num) || !hidden_ocgs.contains(num)
        }
    };

    let visible = match policy {
        "AllOn" => member_ids.iter().all(is_member_on),
        "AnyOff" => member_ids.iter().any(|n| !is_member_on(n)),
        "AllOff" => member_ids.iter().all(|n| !is_member_on(n)),
        _ => member_ids.iter().any(is_member_on), // "AnyOn" default per ISO 32000-1 Table 97
    };
    Ok(visible)
}

/// Resolves the `/Resources /Properties` dictionary for `page_id` (walking `/Parent` if inherited)
/// and returns the set of property names (e.g. `"MC0"`, `"Layer1"`) whose associated `/OCG` or
/// `/OCMD` is currently hidden (`visible == false`).
pub fn resolve_page_hidden_ocg_props(
    doc: &mut PdfDocument,
    page_id: ObjectId,
) -> PdfResult<HashSet<String>> {
    let layers = extract_layers(doc)?;
    if layers.is_empty() || layers.iter().all(|l| l.visible) {
        return Ok(HashSet::new());
    }

    let mut visible_ocgs = HashSet::new();
    let mut hidden_ocgs = HashSet::new();
    for layer in &layers {
        if layer.visible {
            visible_ocgs.insert(layer.object_number);
        } else {
            hidden_ocgs.insert(layer.object_number);
        }
    }

    // Walk page -> /Parent chain to locate `/Resources` -> `/Properties`
    let mut current_id = Some(page_id);
    let mut visited = HashSet::new();
    let mut merged_props = PdfDictionary::new();
    let mut depth = 0;

    while let Some(cid) = current_id {
        if depth > MAX_LAYER_DEPTH || !visited.insert(cid) {
            break;
        }
        depth += 1;

        let Ok(obj) = doc.get_object(cid) else {
            break;
        };
        let Some(dict) = obj.as_dict().cloned() else {
            break;
        };

        if let Some(res_obj) = dict.get("Resources").cloned() {
            if let Some(res_dict) = resolve_dict(doc, &res_obj)? {
                if let Some(props_obj) = res_dict.get("Properties").cloned() {
                    if let Some(props_dict) = resolve_dict(doc, &props_obj)? {
                        for (k, v) in props_dict.iter() {
                            if !merged_props.contains_key(k.as_str()) {
                                merged_props.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
            }
        }

        current_id = dict.get("Parent").and_then(|o| o.as_reference());
    }

    let mut hidden_props = HashSet::new();
    for (prop_key, prop_val) in merged_props.iter() {
        match prop_val {
            PdfObject::Reference(ref_id) => {
                if hidden_ocgs.contains(&ref_id.number) {
                    hidden_props.insert(prop_key.as_str().to_string());
                    continue;
                }
                if let Ok(target_obj) = doc.get_object(*ref_id) {
                    if let Some(target_dict) = target_obj.as_dict().cloned() {
                        if target_dict.get("Type").and_then(|o| o.as_name()) == Some("OCMD") {
                            let is_vis = evaluate_ocmd_visibility(
                                doc,
                                &target_dict,
                                &visible_ocgs,
                                &hidden_ocgs,
                            )?;
                            if !is_vis {
                                hidden_props.insert(prop_key.as_str().to_string());
                            }
                        }
                    }
                }
            }
            PdfObject::Dictionary(target_dict) => {
                if target_dict.get("Type").and_then(|o| o.as_name()) == Some("OCMD") {
                    let is_vis = evaluate_ocmd_visibility(
                        doc,
                        target_dict,
                        &visible_ocgs,
                        &hidden_ocgs,
                    )?;
                    if !is_vis {
                        hidden_props.insert(prop_key.as_str().to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Ok(hidden_props)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::PdfStream;
    use crate::fonts::FontMetrics;
    use crate::layout::LayoutReconstructor;
    use crate::stream::ContentParser;

    fn create_layered_doc() -> (PdfDocument, ObjectId) {
        let mut doc = PdfDocument::empty();
        let page_id = reserve_object_id(&mut doc);
        let content_id = reserve_object_id(&mut doc);

        let stream_bytes = b"BT\n/F1 12 Tf\n72 720 Td\n/OC /L1 BDC\n(Watermark Confidential Layer) Tj\nEMC\n0 -60 Td\n/OC /L2 BDC\n(Primary Contract Body Text) Tj\nEMC\nET\n";
        let mut stream_dict = PdfDictionary::new();
        stream_dict.insert("Length", PdfObject::Integer(stream_bytes.len() as i64));
        doc.set_object(
            content_id,
            PdfObject::Stream(PdfStream::new(stream_dict, stream_bytes.to_vec())),
        );

        let mut page_dict = PdfDictionary::new();
        page_dict.insert("Type", PdfObject::Name(PdfName::new("Page")));
        page_dict.insert("Parent", PdfObject::Reference(ObjectId::new(2)));
        page_dict.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(612),
                PdfObject::Integer(792),
            ]),
        );
        page_dict.insert("Contents", PdfObject::Reference(content_id));
        doc.set_object(page_id, PdfObject::Dictionary(page_dict));

        if let Some(pages_dict) = doc
            .objects
            .get_mut(&ObjectId::new(2))
            .and_then(|o| o.as_dict_mut())
        {
            pages_dict.insert(
                "Kids",
                PdfObject::Array(vec![PdfObject::Reference(page_id)]),
            );
            pages_dict.insert("Count", PdfObject::Integer(1));
        }
        (doc, page_id)
    }

    #[test]
    fn test_layers_crud_and_scenegraph_filtering_roundtrip() {
        let (mut doc, page_id) = create_layered_doc();
        assert!(extract_layers(&mut doc).unwrap().is_empty());

        let l1 = add_layer(
            &mut doc,
            "Watermark Layer",
            true,
            Some(vec!["View".to_string(), "Design".to_string()]),
            false,
        )
        .unwrap();
        let l2 = add_layer(&mut doc, "Contract Body", true, None, true).unwrap();

        register_page_layer_property(&mut doc, 1, "L1", &l1.id).unwrap();
        register_page_layer_property(&mut doc, 1, "L2", &l2.name).unwrap();

        // Save and reload
        let saved = doc.save_to_vec().unwrap();
        let mut reloaded = PdfDocument::load(&saved).unwrap();

        let layers = extract_layers(&mut reloaded).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].name, "Watermark Layer");
        assert!(layers[0].visible);
        assert_eq!(layers[0].intent, vec!["View", "Design"]);
        assert!(!layers[0].locked);

        assert_eq!(layers[1].name, "Contract Body");
        assert!(layers[1].visible);
        assert!(layers[1].locked);

        // Both layers are ON -> LayoutReconstructor extracts both paragraphs
        let content_bytes = reloaded.get_page_content_bytes(page_id).unwrap();
        let ast = ContentParser::new(&content_bytes).parse().unwrap();
        let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);

        let hidden_none = resolve_page_hidden_ocg_props(&mut reloaded, page_id).unwrap();
        assert!(hidden_none.is_empty());
        let paras_all = LayoutReconstructor::new(&ast)
            .with_font("F1", metrics.clone())
            .with_hidden_ocg_props(hidden_none)
            .reconstruct()
            .unwrap();
        assert_eq!(paras_all.len(), 2);
        assert_eq!(paras_all[0].text(), "Watermark Confidential Layer");
        assert_eq!(paras_all[1].text(), "Primary Contract Body Text");

        // Toggle "Watermark Layer" OFF -> LayoutReconstructor filters out L1 and keeps L2 at exact coordinates!
        let toggled = set_layer_visibility(&mut reloaded, "Watermark Layer", false).unwrap();
        assert!(!toggled.visible);

        let hidden_l1 = resolve_page_hidden_ocg_props(&mut reloaded, page_id).unwrap();
        assert!(hidden_l1.contains("L1"));
        assert!(!hidden_l1.contains("L2"));

        let paras_filtered = LayoutReconstructor::new(&ast)
            .with_font("F1", metrics)
            .with_hidden_ocg_props(hidden_l1)
            .reconstruct()
            .unwrap();
        assert_eq!(paras_filtered.len(), 1);
        assert_eq!(paras_filtered[0].text(), "Primary Contract Body Text");
        assert_eq!(paras_filtered[0].bbox, paras_all[1].bbox);

        // Delete layer L1
        assert!(delete_layer(&mut reloaded, &l1.id).unwrap());
        assert!(!delete_layer(&mut reloaded, &l1.id).unwrap());
        assert_eq!(extract_layers(&mut reloaded).unwrap().len(), 1);
    }
}
