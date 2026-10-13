//! Hard Flattening / Burn-In Engine (ISO 32000-1 §12.5.5 & §12.7).
//!
//! Irreversibly compiles interactive AcroForm fields, visual annotations, rubber stamps,
//! vector shapes (`Ink`, `Square`, `Circle`, `Line`, `Polygon`), and `/AP /N` appearance
//! streams directly into the page's content stream (`/Contents`), and permanently purges
//! `/Annots` and `/AcroForm` interactive dictionaries so flattened elements cannot be
//! peeled off, edited, or tampered with.

use std::collections::{HashMap, HashSet};

use crate::annots::reader::extract_page_annotations;
use crate::annots::types::{Annotation, AnnotationSubtype};
use crate::cos::{ObjectId, PdfDictionary, PdfName, PdfObject, PdfStream};
use crate::error::{PdfError, PdfResult};
use crate::forms::reader::extract_document_forms;
use crate::forms::types::{FormField, FormFieldType};
use crate::layout::geometry::Rect;
use crate::PdfDocument;

/// Summary metrics returned after executing a permanent hard-flatten operation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HardFlattenReport {
    /// Number of pages whose content streams were updated.
    pub pages_flattened: usize,
    /// Number of interactive AcroForm fields burned into page content streams.
    pub forms_flattened: usize,
    /// Number of annotations (highlights, stamps, shapes, links, etc.) burned in and removed.
    pub annotations_flattened: usize,
    /// Number of normal appearance streams (`/AP /N`) compiled into page content streams.
    pub appearance_streams_compiled: usize,
    /// Number of interactive COS dictionaries (`/Annot`, `/Widget`, `/AcroForm`) purged from the document.
    pub objects_purged: usize,
}

/// Resolves a `PdfObject` to a `PdfDictionary` if it is a direct dictionary or an indirect reference.
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

/// Resolves a `PdfObject` to a `Vec<PdfObject>` if it is a direct array or an indirect reference.
fn resolve_array(doc: &mut PdfDocument, obj: &PdfObject) -> PdfResult<Option<Vec<PdfObject>>> {
    match obj {
        PdfObject::Array(a) => Ok(Some(a.clone())),
        PdfObject::Reference(id) => {
            let resolved = doc.get_object(*id)?;
            Ok(resolved.as_array().map(|a| a.to_vec()))
        }
        _ => Ok(None),
    }
}

/// Extracts a `[min_x, min_y, max_x, max_y]` rectangle from a PDF array object.
fn parse_rect(arr: &[PdfObject]) -> Option<Rect> {
    if arr.len() < 4 {
        return None;
    }
    let x1 = arr[0].as_f64()?;
    let y1 = arr[1].as_f64()?;
    let x2 = arr[2].as_f64()?;
    let y2 = arr[3].as_f64()?;
    Some(Rect::new(x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)))
}

/// Escapes parentheses and backslashes for PDF literal strings.
fn escape_pdf_literal(s: &str) -> String {
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

/// Resolves the normal appearance stream (`/AP /N`) object ID for an annotation dictionary,
/// handling both direct stream references and appearance-state subdictionaries keyed by `/AS`.
fn resolve_normal_appearance_stream_id(
    doc: &mut PdfDocument,
    annot_dict: &PdfDictionary,
) -> PdfResult<Option<ObjectId>> {
    let Some(ap_obj) = annot_dict.get("AP").cloned() else {
        return Ok(None);
    };
    let Some(ap_dict) = resolve_dict(doc, &ap_obj)? else {
        return Ok(None);
    };
    let Some(n_obj) = ap_dict.get("N").cloned() else {
        return Ok(None);
    };

    match n_obj {
        PdfObject::Reference(id) => {
            let target = doc.get_object(id)?;
            match target {
                PdfObject::Stream(_) => Ok(Some(id)),
                PdfObject::Dictionary(state_dict) => {
                    if let Some(as_name) = annot_dict.get("AS").and_then(|o| o.as_name()) {
                        if let Some(PdfObject::Reference(sub_id)) = state_dict.get(as_name) {
                            return Ok(Some(*sub_id));
                        }
                    }
                    for (_, val) in state_dict.iter() {
                        if let PdfObject::Reference(sub_id) = val {
                            return Ok(Some(*sub_id));
                        }
                    }
                    Ok(None)
                }
                _ => Ok(None),
            }
        }
        PdfObject::Dictionary(state_dict) => {
            if let Some(as_name) = annot_dict.get("AS").and_then(|o| o.as_name()) {
                if let Some(PdfObject::Reference(sub_id)) = state_dict.get(as_name) {
                    return Ok(Some(*sub_id));
                }
            }
            for (_, val) in state_dict.iter() {
                if let PdfObject::Reference(sub_id) = val {
                    return Ok(Some(*sub_id));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Checks whether an annotation is marked Hidden (bit 2 = 2) or Invisible (bit 1 = 1) in `/F`.
fn is_annotation_hidden(annot_dict: &PdfDictionary) -> bool {
    let flags = annot_dict.get("F").and_then(|o| o.as_i64()).unwrap_or(0);
    (flags & 1) != 0 || (flags & 2) != 0
}

/// Permanently flattens all (or selected pages') interactive AcroForm fields, visual annotations,
/// stamps, shapes, and `/AP /N` appearance streams directly into page content streams, and purges
/// `/Annots` and `/AcroForm` dictionaries from the document.
///
/// `page_indices` is an optional slice of 0-based page indices. When `None`, every page in the
/// document is flattened and `/AcroForm` is completely removed from `/Catalog`.
pub fn flatten_document_permanently(
    doc: &mut PdfDocument,
    page_indices: Option<&[usize]>,
) -> PdfResult<HardFlattenReport> {
    let pages = doc.get_pages()?;
    let target_indices: Vec<usize> = match page_indices {
        Some(indices) => {
            let mut v = Vec::with_capacity(indices.len());
            for &idx in indices {
                if idx >= pages.len() {
                    return Err(PdfError::InvalidPageNumber {
                        page: idx + 1,
                        total: pages.len(),
                    });
                }
                if !v.contains(&idx) {
                    v.push(idx);
                }
            }
            v
        }
        None => (0..pages.len()).collect(),
    };

    let all_fields = extract_document_forms(doc)?;
    let mut fields_by_page: HashMap<ObjectId, Vec<FormField>> = HashMap::new();
    for field in all_fields {
        fields_by_page.entry(field.page_id).or_default().push(field);
    }

    let mut report = HardFlattenReport::default();
    let mut purged_ids: HashSet<ObjectId> = HashSet::new();

    for page_idx in &target_indices {
        let page_id = pages[*page_idx];
        let page_fields = fields_by_page.remove(&page_id).unwrap_or_default();
        let page_annots = extract_page_annotations(doc, *page_idx).unwrap_or_default();

        // Also inspect raw `/Annots` references on the page so we catch any annotation subtype
        let page_dict_opt = doc.get_object(page_id)?.as_dict().cloned();
        let Some(mut page_dict) = page_dict_opt else {
            continue;
        };

        let raw_annot_ids: Vec<ObjectId> = if let Some(annots_obj) = page_dict.get("Annots").cloned()
        {
            resolve_array(doc, &annots_obj)?
                .unwrap_or_default()
                .into_iter()
                .filter_map(|o| o.as_reference())
                .collect()
        } else {
            Vec::new()
        };

        if page_fields.is_empty() && page_annots.is_empty() && raw_annot_ids.is_empty() {
            continue;
        }

        let field_id_set: HashSet<ObjectId> = page_fields.iter().map(|f| f.id).collect();
        let mut append_ops = String::from("\n% --- Hard Flattened Overlay Burn-In ---\n");
        let mut needs_helv_font = false;
        let mut needs_stamp_font = false;
        let mut needs_extg = false;
        let mut xobject_bindings: Vec<(String, ObjectId)> = Vec::new();
        let mut ap_ids_to_purge: Vec<ObjectId> = Vec::new();
        let mut handled_annot_ids: HashSet<ObjectId> = HashSet::new();

        // 1. Burn in interactive AcroForm fields on this page
        for field in &page_fields {
            handled_annot_ids.insert(field.id);
            let width = field.rect.width().max(4.0);
            let height = field.rect.height().max(4.0);
            let llx = field.rect.min_x;
            let lly = field.rect.min_y;

            // Check if field widget has a normal appearance stream (/AP /N)
            let ap_id_opt = if let Ok(f_obj) = doc.get_object(field.id) {
                if let Some(f_dict) = f_obj.as_dict().cloned() {
                    resolve_normal_appearance_stream_id(doc, &f_dict)?
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(ap_id) = ap_id_opt {
                report.appearance_streams_compiled += 1;
                if matches!(field.field_type, FormFieldType::Signature) || field.value.is_empty() {
                    if let Some(do_ops) =
                        compile_ap_xobject_do(doc, ap_id, field.rect, &mut xobject_bindings)?
                    {
                        append_ops.push_str(&do_ops);
                    }
                } else {
                    ap_ids_to_purge.push(ap_id);
                }
            }

            match field.field_type {
                FormFieldType::Text | FormFieldType::Choice => {
                    if !field.value.is_empty() {
                        needs_helv_font = true;
                        let font_size = (height * 0.65).clamp(8.0, 14.0);
                        let baseline_y = (height - font_size) * 0.5;
                        let escaped = escape_pdf_literal(&field.value);
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

            report.forms_flattened += 1;
            purged_ids.insert(field.id);
        }

        // 2. Burn in high-level extracted annotations (Highlight, Underline, StrikeOut, Stamp, Shape, Link, etc.)
        for annot in &page_annots {
            if field_id_set.contains(&annot.id) {
                continue;
            }
            handled_annot_ids.insert(annot.id);

            let annot_dict_opt = doc
                .get_object(annot.id)
                .ok()
                .and_then(|o| o.as_dict().cloned());
            if let Some(ref ad) = annot_dict_opt {
                if is_annotation_hidden(ad) {
                    purged_ids.insert(annot.id);
                    report.annotations_flattened += 1;
                    continue;
                }
            }

            let ap_id_opt = if let Some(ref ad) = annot_dict_opt {
                resolve_normal_appearance_stream_id(doc, ad)?
            } else {
                None
            };

            if let Some(ap_id) = ap_id_opt {
                report.appearance_streams_compiled += 1;
                // For Stamp annotations, we synthesize page-space text+border operators below so the
                // stamp text is directly searchable in the page's SceneGraph, and purge the XObject.
                // For Shapes (Ink, Square, Circle, Line, Polygon) and custom appearances, we compile
                // the Form XObject into /Resources /XObject and invoke `cm` + `Do`.
                if matches!(annot.subtype, AnnotationSubtype::Stamp) {
                    ap_ids_to_purge.push(ap_id);
                } else if let Some(do_ops) =
                    compile_ap_xobject_do(doc, ap_id, annot.rect, &mut xobject_bindings)?
                {
                    append_ops.push_str(&do_ops);
                    report.annotations_flattened += 1;
                    purged_ids.insert(annot.id);
                    continue;
                }
            }

            render_annotation_fallback_ops(
                annot,
                &mut append_ops,
                &mut needs_extg,
                &mut needs_stamp_font,
                &mut needs_helv_font,
            );
            report.annotations_flattened += 1;
            purged_ids.insert(annot.id);
        }

        // 3. Burn in any remaining raw `/Annots` entries not covered above
        for raw_id in raw_annot_ids {
            if handled_annot_ids.contains(&raw_id) {
                continue;
            }
            handled_annot_ids.insert(raw_id);
            if let Ok(raw_obj) = doc.get_object(raw_id) {
                if let Some(raw_dict) = raw_obj.as_dict().cloned() {
                    if !is_annotation_hidden(&raw_dict) {
                        if let Some(ap_id) = resolve_normal_appearance_stream_id(doc, &raw_dict)? {
                            let rect = raw_dict
                                .get("Rect")
                                .and_then(|r| r.as_array())
                                .and_then(parse_rect)
                                .unwrap_or_else(|| Rect::new(0.0, 0.0, 100.0, 30.0));
                            report.appearance_streams_compiled += 1;
                            if let Some(do_ops) =
                                compile_ap_xobject_do(doc, ap_id, rect, &mut xobject_bindings)?
                            {
                                append_ops.push_str(&do_ops);
                            }
                        }
                    }
                    let subtype_name = raw_dict.get("Subtype").and_then(|s| s.as_name());
                    if subtype_name == Some("Widget") {
                        report.forms_flattened += 1;
                    } else {
                        report.annotations_flattened += 1;
                    }
                }
            }
            purged_ids.insert(raw_id);
        }

        // 4. Write updated `/Contents` stream and `/Resources` on the page
        let mut existing_bytes = doc.get_page_content_bytes(page_id).unwrap_or_default();
        existing_bytes.extend_from_slice(append_ops.as_bytes());

        let new_contents_id = doc.alloc_object_id();
        let mut stream_dict = PdfDictionary::new();
        stream_dict.insert("Length", PdfObject::Integer(existing_bytes.len() as i64));
        doc.set_object(
            new_contents_id,
            PdfObject::Stream(PdfStream::new(stream_dict, existing_bytes)),
        );
        page_dict.insert("Contents", PdfObject::Reference(new_contents_id));

        // Remove `/Annots` completely from the page dictionary
        page_dict.remove("Annots");
        doc.set_object(page_id, PdfObject::Dictionary(page_dict));

        // Update page `/Resources` (Fonts, ExtGState, and compiled Form XObjects)
        update_page_flatten_resources(
            doc,
            page_id,
            needs_helv_font,
            needs_stamp_font,
            needs_extg,
            &xobject_bindings,
        )?;

        for ap_id in ap_ids_to_purge {
            purged_ids.insert(ap_id);
        }

        report.pages_flattened += 1;
    }

    // 5. Purge `/AcroForm` from `/Catalog` (or prune flattened fields if partial pages were requested)
    purge_acroform_after_flatten(doc, page_indices.is_none(), &purged_ids, &mut report)?;

    // 6. Remove all purged interactive objects from `doc.objects`
    for pid in purged_ids {
        if doc.objects.remove(&pid).is_some() {
            report.objects_purged += 1;
        }
    }

    Ok(report)
}

/// Compiles a normal appearance stream (`/AP /N` Form XObject) into a page `q ... cm /FlatAP_<id> Do Q`
/// invocation and registers the XObject binding.
fn compile_ap_xobject_do(
    doc: &mut PdfDocument,
    ap_id: ObjectId,
    target_rect: Rect,
    xobject_bindings: &mut Vec<(String, ObjectId)>,
) -> PdfResult<Option<String>> {
    let Ok(ap_obj) = doc.get_object(ap_id) else {
        return Ok(None);
    };
    let PdfObject::Stream(ap_stream) = ap_obj else {
        return Ok(None);
    };

    let bbox = ap_stream
        .dict
        .get("BBox")
        .and_then(|o| o.as_array())
        .and_then(parse_rect)
        .unwrap_or_else(|| Rect::new(0.0, 0.0, target_rect.width(), target_rect.height()));

    let bbox_w = bbox.width();
    let bbox_h = bbox.height();
    let sx = if bbox_w > 0.001 {
        target_rect.width() / bbox_w
    } else {
        1.0
    };
    let sy = if bbox_h > 0.001 {
        target_rect.height() / bbox_h
    } else {
        1.0
    };
    let tx = target_rect.min_x - bbox.min_x * sx;
    let ty = target_rect.min_y - bbox.min_y * sy;

    let xobj_name = format!("FlatAP_{}", ap_id.number);
    if !xobject_bindings.iter().any(|(n, _)| n == &xobj_name) {
        xobject_bindings.push((xobj_name.clone(), ap_id));
    }

    Ok(Some(format!(
        "q\n{:.4} 0 0 {:.4} {:.2} {:.2} cm\n/{} Do\nQ\n",
        sx, sy, tx, ty, xobj_name
    )))
}

/// Emits page-space vector and text operators for annotations (`Highlight`, `Underline`, `StrikeOut`,
/// `Stamp`, `FreeText`) so they are permanently baked into `/Contents`.
fn render_annotation_fallback_ops(
    annot: &Annotation,
    append_ops: &mut String,
    needs_extg: &mut bool,
    needs_stamp_font: &mut bool,
    needs_helv_font: &mut bool,
) {
    let rect = annot.rect;
    let width = rect.width().max(1.0);
    let height = rect.height().max(1.0);
    let c = annot.color.unwrap_or([1.0, 0.92, 0.23]);

    match annot.subtype {
        AnnotationSubtype::Highlight => {
            *needs_extg = true;
            append_ops.push_str("q\n/GS_HL gs\n");
            append_ops.push_str(&format!("{:.3} {:.3} {:.3} rg\n", c[0], c[1], c[2]));
            append_ops.push_str(&format!(
                "{:.2} {:.2} {:.2} {:.2} re f\nQ\n",
                rect.min_x, rect.min_y, width, height
            ));
        }
        AnnotationSubtype::Underline => {
            append_ops.push_str("q\n");
            append_ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n1.5 w\n", c[0], c[1], c[2]));
            append_ops.push_str(&format!(
                "{:.2} {:.2} m {:.2} {:.2} l S\nQ\n",
                rect.min_x,
                rect.min_y + 1.0,
                rect.max_x,
                rect.min_y + 1.0
            ));
        }
        AnnotationSubtype::StrikeOut => {
            let y_mid = rect.min_y + (height / 2.0);
            append_ops.push_str("q\n");
            append_ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n1.5 w\n", c[0], c[1], c[2]));
            append_ops.push_str(&format!(
                "{:.2} {:.2} m {:.2} {:.2} l S\nQ\n",
                rect.min_x, y_mid, rect.max_x, y_mid
            ));
        }
        AnnotationSubtype::Stamp => {
            *needs_extg = true;
            *needs_stamp_font = true;
            let text = annot
                .contents
                .clone()
                .or_else(|| annot.stamp_type.as_ref().map(|s| s.text()))
                .unwrap_or_else(|| "APPROVED".to_string());

            append_ops.push_str("q\n/GS_Stamp gs\n");
            append_ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n", c[0], c[1], c[2]));
            append_ops.push_str(&format!("{:.3} {:.3} {:.3} rg\n", c[0], c[1], c[2]));

            let m = 2.0;
            append_ops.push_str(&format!(
                "2.5 w\n{:.2} {:.2} {:.2} {:.2} re S\n",
                rect.min_x + m,
                rect.min_y + m,
                (width - m * 2.0).max(1.0),
                (height - m * 2.0).max(1.0)
            ));

            let im = 5.0;
            append_ops.push_str(&format!(
                "0.8 w\n{:.2} {:.2} {:.2} {:.2} re S\n",
                rect.min_x + im,
                rect.min_y + im,
                (width - im * 2.0).max(1.0),
                (height - im * 2.0).max(1.0)
            ));

            let char_count = text.len().max(1) as f64;
            let font_size = (height * 0.45)
                .min((width * 0.8) / (char_count * 0.65))
                .clamp(9.0, 24.0);
            let est_width = char_count * font_size * 0.58;
            let text_x = rect.min_x + ((width - est_width) / 2.0).max(4.0);
            let text_y = rect.min_y + (height - font_size) / 2.0 + (font_size * 0.22);

            append_ops.push_str(&format!(
                "BT\n/F_Stamp {:.1} Tf\n{:.2} {:.2} Td\n({}) Tj\nET\nQ\n",
                font_size,
                text_x,
                text_y,
                escape_pdf_literal(&text)
            ));
        }
        AnnotationSubtype::Other => {
            if let Some(ref text) = annot.contents {
                if !text.is_empty() {
                    *needs_helv_font = true;
                    let font_size = (height * 0.6).clamp(8.0, 14.0);
                    let baseline_y = (height - font_size) * 0.5;
                    append_ops.push_str(&format!(
                        "q\nBT\n/F1 {:.1} Tf\n{:.3} {:.3} {:.3} rg\n{:.2} {:.2} Td\n({}) Tj\nET\nQ\n",
                        font_size,
                        c[0],
                        c[1],
                        c[2],
                        rect.min_x + 2.0,
                        rect.min_y + baseline_y,
                        escape_pdf_literal(text)
                    ));
                }
            }
        }
        _ => {}
    }
}

/// Ensures required `/Font`, `/ExtGState`, and `/XObject` entries exist in the page's `/Resources`.
fn update_page_flatten_resources(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    needs_helv_font: bool,
    needs_stamp_font: bool,
    needs_extg: bool,
    xobject_bindings: &[(String, ObjectId)],
) -> PdfResult<()> {
    if !needs_helv_font && !needs_stamp_font && !needs_extg && xobject_bindings.is_empty() {
        return Ok(());
    }

    let page_obj = doc.get_object(page_id)?;
    let Some(mut page_dict) = page_obj.as_dict().cloned() else {
        return Ok(());
    };

    let existing_res = page_dict.get("Resources").cloned();
    let mut res_dict = if let Some(ref obj) = existing_res {
        resolve_dict(doc, obj)?.unwrap_or_default()
    } else {
        PdfDictionary::new()
    };

    if needs_helv_font || needs_stamp_font {
        let existing_font = res_dict.get("Font").cloned();
        let mut font_dict = if let Some(ref obj) = existing_font {
            resolve_dict(doc, obj)?.unwrap_or_default()
        } else {
            PdfDictionary::new()
        };

        if needs_helv_font && !font_dict.contains_key("F1") {
            let mut f1 = PdfDictionary::new();
            f1.insert("Type", PdfObject::Name(PdfName::new("Font")));
            f1.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
            f1.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica")));
            font_dict.insert("F1", PdfObject::Dictionary(f1));
        }

        if needs_stamp_font && !font_dict.contains_key("F_Stamp") {
            let mut fst = PdfDictionary::new();
            fst.insert("Type", PdfObject::Name(PdfName::new("Font")));
            fst.insert("Subtype", PdfObject::Name(PdfName::new("Type1")));
            fst.insert("BaseFont", PdfObject::Name(PdfName::new("Helvetica-Bold")));
            font_dict.insert("F_Stamp", PdfObject::Dictionary(fst));
        }

        res_dict.insert("Font", PdfObject::Dictionary(font_dict));
    }

    if needs_extg {
        let existing_extg = res_dict.get("ExtGState").cloned();
        let mut extg_dict = if let Some(ref obj) = existing_extg {
            resolve_dict(doc, obj)?.unwrap_or_default()
        } else {
            PdfDictionary::new()
        };

        if !extg_dict.contains_key("GS_HL") {
            let mut gs_hl = PdfDictionary::new();
            gs_hl.insert("Type", PdfObject::Name(PdfName::new("ExtGState")));
            gs_hl.insert("ca", PdfObject::Real(0.45));
            gs_hl.insert("CA", PdfObject::Real(0.45));
            gs_hl.insert("BM", PdfObject::Name(PdfName::new("Multiply")));
            extg_dict.insert("GS_HL", PdfObject::Dictionary(gs_hl));
        }

        if !extg_dict.contains_key("GS_Stamp") {
            let mut gs_st = PdfDictionary::new();
            gs_st.insert("Type", PdfObject::Name(PdfName::new("ExtGState")));
            gs_st.insert("ca", PdfObject::Real(0.85));
            gs_st.insert("CA", PdfObject::Real(0.85));
            extg_dict.insert("GS_Stamp", PdfObject::Dictionary(gs_st));
        }

        res_dict.insert("ExtGState", PdfObject::Dictionary(extg_dict));
    }

    if !xobject_bindings.is_empty() {
        let existing_xobj = res_dict.get("XObject").cloned();
        let mut xobj_dict = if let Some(ref obj) = existing_xobj {
            resolve_dict(doc, obj)?.unwrap_or_default()
        } else {
            PdfDictionary::new()
        };

        for (name, target_id) in xobject_bindings {
            xobj_dict.insert(name.as_str(), PdfObject::Reference(*target_id));
        }

        res_dict.insert("XObject", PdfObject::Dictionary(xobj_dict));
    }

    page_dict.insert("Resources", PdfObject::Dictionary(res_dict));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));
    Ok(())
}

/// Removes or updates `/AcroForm` in `/Catalog` after flattening.
fn purge_acroform_after_flatten(
    doc: &mut PdfDocument,
    flatten_all_pages: bool,
    purged_ids: &HashSet<ObjectId>,
    report: &mut HardFlattenReport,
) -> PdfResult<()> {
    let Some(root_ref) = doc.xref.trailer.get("Root").and_then(|r| r.as_reference()) else {
        return Ok(());
    };
    let root_obj = doc.get_object(root_ref)?;
    let Some(mut catalog) = root_obj.as_dict().cloned() else {
        return Ok(());
    };

    let Some(acro_obj) = catalog.get("AcroForm").cloned() else {
        return Ok(());
    };

    if flatten_all_pages {
        if let PdfObject::Reference(acro_id) = acro_obj {
            if let Ok(PdfObject::Dictionary(acro_dict)) = doc.get_object(acro_id) {
                if let Some(fields_obj) = acro_dict.get("Fields") {
                    if let Ok(Some(fields_arr)) = resolve_array(doc, fields_obj) {
                        for item in fields_arr {
                            if let PdfObject::Reference(fid) = item {
                                if doc.objects.remove(&fid).is_some() {
                                    report.objects_purged += 1;
                                }
                            }
                        }
                    }
                }
            }
            if doc.objects.remove(&acro_id).is_some() {
                report.objects_purged += 1;
            }
        }
        catalog.remove("AcroForm");
        doc.set_object(root_ref, PdfObject::Dictionary(catalog));
    } else if let Some(mut acro_dict) = resolve_dict(doc, &acro_obj)? {
        if let Some(fields_obj) = acro_dict.get("Fields").cloned() {
            if let Some(mut fields_arr) = resolve_array(doc, &fields_obj)? {
                fields_arr.retain(|item| {
                    item.as_reference()
                        .map(|id| !purged_ids.contains(&id))
                        .unwrap_or(true)
                });
                if fields_arr.is_empty() {
                    if let PdfObject::Reference(acro_id) = acro_obj {
                        if doc.objects.remove(&acro_id).is_some() {
                            report.objects_purged += 1;
                        }
                    }
                    catalog.remove("AcroForm");
                } else {
                    acro_dict.insert("Fields", PdfObject::Array(fields_arr));
                    if let PdfObject::Reference(acro_id) = acro_obj {
                        doc.set_object(acro_id, PdfObject::Dictionary(acro_dict));
                    } else {
                        catalog.insert("AcroForm", PdfObject::Dictionary(acro_dict));
                    }
                }
                doc.set_object(root_ref, PdfObject::Dictionary(catalog));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annots::{add_shape, add_stamp, add_text_markup, ShapeKind, ShapeStyle, StampType};
    use crate::forms::{create_form_field, FormFieldCreateOptions, FormFieldType};
    use crate::layout::LayoutReconstructor;
    use crate::stream::ContentParser;

    fn create_test_doc() -> PdfDocument {
        let mut doc = PdfDocument::empty();
        let page_id = ObjectId::new(3);
        let content_id = ObjectId::new(4);

        let stream_bytes = b"BT\n/F1 12 Tf\n72 720 Td\n(Contract Header) Tj\nET\n";
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
        doc
    }

    #[test]
    fn test_hard_flatten_burns_in_forms_stamps_shapes_and_purges_interactive_dicts() {
        let mut doc = create_test_doc();

        // 1. Add an AcroForm Text field and Checkbox field
        create_form_field(
            &mut doc,
            1,
            &FormFieldCreateOptions {
                name: "ClientName".to_string(),
                field_type: FormFieldType::Text,
                rect: Rect::new(72.0, 640.0, 260.0, 664.0),
                value: Some("Roberto Garduno".to_string()),
                default_value: None,
                alt_name: None,
                options: None,
                is_read_only: false,
                is_required: true,
                is_multiline: false,
                max_length: None,
                font_size: Some(12.0),
            },
        )
        .unwrap();

        // 2. Add a Stamp annotation (with /AP /N stream)
        add_stamp(
            &mut doc,
            0,
            StampType::Approved,
            Some(Rect::new(350.0, 640.0, 510.0, 690.0)),
            Some("APPROVED BY LEGAL"),
            None,
            None,
        )
        .unwrap();

        // 3. Add a Vector Shape annotation (with /AP /N Form XObject stream)
        add_shape(
            &mut doc,
            0,
            ShapeKind::Square,
            &[[72.0, 500.0], [200.0, 560.0]],
            ShapeStyle {
                stroke: [0.1, 0.2, 0.8],
                fill: Some([0.9, 0.95, 1.0]),
                line_width: 2.0,
                opacity: 0.9,
                line_ending: None,
            },
        )
        .unwrap();

        // 4. Add a Highlight annotation
        add_text_markup(
            &mut doc,
            0,
            AnnotationSubtype::Highlight,
            Rect::new(72.0, 715.0, 180.0, 735.0),
            None,
            Some([1.0, 0.9, 0.2]),
            None,
            Some("Important header"),
        )
        .unwrap();

        assert_eq!(extract_document_forms(&mut doc).unwrap().len(), 1);
        assert_eq!(extract_page_annotations(&mut doc, 0).unwrap().len(), 3);

        // Execute permanent hard flatten
        let report = flatten_document_permanently(&mut doc, None).unwrap();
        assert_eq!(report.pages_flattened, 1);
        assert_eq!(report.forms_flattened, 1);
        assert_eq!(report.annotations_flattened, 3);
        assert!(report.appearance_streams_compiled >= 2);
        assert!(report.objects_purged >= 4);

        // Save and reload
        let saved = doc.save_to_vec().unwrap();
        let mut reloaded = PdfDocument::load(&saved).unwrap();

        // Interactive forms and annotations are 100% gone
        assert!(extract_document_forms(&mut reloaded).unwrap().is_empty());
        assert!(extract_page_annotations(&mut reloaded, 0)
            .unwrap()
            .is_empty());

        // Burned-in text ("Roberto Garduno" and "APPROVED BY LEGAL") is now part of the page's content stream & SceneGraph!
        let page_id = reloaded.get_pages().unwrap()[0];
        let content_bytes = reloaded.get_page_content_bytes(page_id).unwrap();
        let content_str = String::from_utf8_lossy(&content_bytes);
        assert!(content_str.contains("Roberto Garduno"));
        assert!(content_str.contains("APPROVED BY LEGAL"));
        assert!(content_str.contains("FlatAP_"));

        let ast = ContentParser::new(&content_bytes).parse().unwrap();
        let paras = LayoutReconstructor::new(&ast).reconstruct().unwrap();
        let all_text: String = paras
            .iter()
            .map(|p| p.text())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(all_text.contains("Contract Header"));
        assert!(all_text.contains("Roberto Garduno"));
        assert!(all_text.contains("APPROVED BY LEGAL"));
    }
}
