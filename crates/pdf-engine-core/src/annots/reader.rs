//! Annotation extraction and reader according to ISO 32000-1 §12.5.

use crate::annots::types::{Annotation, AnnotationSubtype, LinkAction, StampType};
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::layout::geometry::Rect;

/// Extracts all non-widget annotations declared on a specific page.
pub fn extract_page_annotations(
    doc: &mut PdfDocument,
    page_index: usize,
) -> PdfResult<Vec<Annotation>> {
    let pages = doc.get_pages()?;
    let page_id = pages
        .get(page_index)
        .copied()
        .ok_or_else(|| PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        })?;

    let page_obj = doc.get_object(page_id)?;
    let page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => return Ok(Vec::new()),
    };

    let annots_array = match page_dict.get("Annots") {
        Some(PdfObject::Array(arr)) => arr.clone(),
        Some(PdfObject::Reference(id)) => match doc.get_object(*id)? {
            PdfObject::Array(arr) => arr,
            _ => return Ok(Vec::new()),
        },
        _ => return Ok(Vec::new()),
    };

    let mut result = Vec::new();

    for item in annots_array {
        let (annot_id, annot_dict) = match item {
            PdfObject::Reference(id) => match doc.get_object(id)? {
                PdfObject::Dictionary(d) => (id, d),
                _ => continue,
            },
            _ => continue,
        };

        // Skip Form / Widget annotations (handled separately by forms module)
        let subtype_name = annot_dict
            .get("Subtype")
            .and_then(|s| s.as_name())
            .unwrap_or("");

        if subtype_name == "Widget" {
            continue;
        }

        let subtype = AnnotationSubtype::from_pdf_name(subtype_name);

        // Parse Rect [llx lly urx ury]
        let rect = match annot_dict.get("Rect") {
            Some(PdfObject::Array(r_arr)) if r_arr.len() >= 4 => {
                let x0 = r_arr[0].as_f64().unwrap_or(0.0);
                let y0 = r_arr[1].as_f64().unwrap_or(0.0);
                let x1 = r_arr[2].as_f64().unwrap_or(0.0);
                let y1 = r_arr[3].as_f64().unwrap_or(0.0);
                Rect::new(x0, y0, x1, y1)
            }
            _ => Rect::new(0.0, 0.0, 0.0, 0.0),
        };

        // Parse Color /C [r g b]
        let color = annot_dict.get("C").and_then(|c_obj| match c_obj {
            PdfObject::Array(c_arr) if c_arr.len() >= 3 => Some([
                c_arr[0].as_f64().unwrap_or(0.0),
                c_arr[1].as_f64().unwrap_or(0.0),
                c_arr[2].as_f64().unwrap_or(0.0),
            ]),
            _ => None,
        });

        // Parse Opacity /CA
        let opacity = annot_dict.get("CA").and_then(|ca| ca.as_f64()).unwrap_or(
            if subtype == AnnotationSubtype::Highlight {
                0.4
            } else {
                1.0
            },
        );

        // Parse Contents /Contents
        let contents = annot_dict.get("Contents").and_then(|c| match c {
            PdfObject::String(s) => Some(s.to_string_lossy()),
            _ => None,
        });

        // Parse Link Action (/A or /Dest)
        let link_action = if subtype == AnnotationSubtype::Link {
            parse_link_action(doc, &annot_dict, &pages)
        } else {
            None
        };

        // Parse Stamp Info
        let (stamp_type, date_str) = if subtype == AnnotationSubtype::Stamp {
            let name_str = annot_dict
                .get("Name")
                .and_then(|n| n.as_name())
                .unwrap_or("");
            let text = contents.clone().unwrap_or_else(|| name_str.to_string());
            let st = StampType::from_name_or_text(&text);
            let date = annot_dict.get("M").and_then(|m| match m {
                PdfObject::String(s) => Some(s.to_string_lossy()),
                _ => None,
            });
            (Some(st), date)
        } else {
            (None, None)
        };

        let border_width = border_width_of(&annot_dict);
        let fill_color = rgb_array(annot_dict.get("IC"));
        let points = shape_points(subtype, &annot_dict);
        let line_ending = line_ending_of(&annot_dict);

        result.push(Annotation {
            id: annot_id,
            page_index,
            page_id,
            subtype,
            rect,
            color,
            opacity,
            contents,
            link_action,
            stamp_type,
            date_str,
            border_width,
            fill_color,
            points,
            line_ending,
        });
    }

    Ok(result)
}

/// Extracts all non-widget annotations across all pages in the document.
pub fn extract_all_annotations(doc: &mut PdfDocument) -> PdfResult<Vec<Annotation>> {
    let pages = doc.get_pages()?;
    let count = pages.len();
    let mut all = Vec::new();
    for page_idx in 0..count {
        let mut page_annots = extract_page_annotations(doc, page_idx)?;
        all.append(&mut page_annots);
    }
    Ok(all)
}

fn rgb_array(obj: Option<&PdfObject>) -> Option<[f64; 3]> {
    match obj {
        Some(PdfObject::Array(values)) if values.len() >= 3 => Some([
            values[0].as_f64().unwrap_or(0.0),
            values[1].as_f64().unwrap_or(0.0),
            values[2].as_f64().unwrap_or(0.0),
        ]),
        _ => None,
    }
}

fn border_width_of(dict: &PdfDictionary) -> f64 {
    let Some(style) = dict.get("BS") else {
        return 1.0;
    };
    let style = match style {
        PdfObject::Dictionary(inner) => inner,
        _ => return 1.0,
    };
    style
        .get("W")
        .and_then(|width| width.as_f64())
        .unwrap_or(1.0)
}

fn pair_list(obj: &PdfObject) -> Vec<[f64; 2]> {
    let PdfObject::Array(values) = obj else {
        return Vec::new();
    };
    let mut points = Vec::new();
    let mut index = 0;
    while index + 1 < values.len() {
        let x = values[index].as_f64().unwrap_or(0.0);
        let y = values[index + 1].as_f64().unwrap_or(0.0);
        points.push([x, y]);
        index += 2;
    }
    points
}

fn shape_points(subtype: AnnotationSubtype, dict: &PdfDictionary) -> Vec<[f64; 2]> {
    match subtype {
        AnnotationSubtype::Ink => {
            let Some(PdfObject::Array(strokes)) = dict.get("InkList") else {
                return Vec::new();
            };
            strokes.first().map(pair_list).unwrap_or_default()
        }
        AnnotationSubtype::Line => dict.get("L").map(pair_list).unwrap_or_default(),
        AnnotationSubtype::Polygon => dict.get("Vertices").map(pair_list).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn line_ending_of(dict: &PdfDictionary) -> Option<String> {
    let PdfObject::Array(values) = dict.get("LE")? else {
        return None;
    };
    let name = values.last().and_then(|item| item.as_name())?;
    if name == "None" {
        None
    } else {
        Some(name.to_string())
    }
}

/// Resolves link actions from an annotation dictionary.
fn parse_link_action(
    doc: &mut PdfDocument,
    dict: &PdfDictionary,
    pages: &[ObjectId],
) -> Option<LinkAction> {
    // 1. Direct /A dictionary
    if let Some(action_obj) = dict.get("A") {
        let action_dict: Option<PdfDictionary> = match action_obj {
            PdfObject::Dictionary(d) => Some(d.clone()),
            PdfObject::Reference(id) => match doc.get_object(*id).ok()? {
                PdfObject::Dictionary(d) => Some(d),
                _ => None,
            },
            _ => None,
        };

        if let Some(a_dict) = action_dict {
            let s_type = a_dict.get("S").and_then(|s| s.as_name()).unwrap_or("");
            if s_type == "URI" {
                if let Some(uri_obj) = a_dict.get("URI") {
                    let uri_str = match uri_obj {
                        PdfObject::String(s) => s.to_string_lossy(),
                        _ => String::new(),
                    };
                    if !uri_str.is_empty() {
                        return Some(LinkAction::Uri(uri_str));
                    }
                }
            } else if s_type == "GoTo" {
                if let Some(dest_obj) = a_dict.get("D") {
                    if let Some(target_idx) = resolve_dest_page_index(dest_obj, pages) {
                        return Some(LinkAction::GoTo(target_idx));
                    }
                }
            }
        }
    }

    // 2. Direct /Dest array
    if let Some(dest_obj) = dict.get("Dest") {
        if let Some(target_idx) = resolve_dest_page_index(dest_obj, pages) {
            return Some(LinkAction::GoTo(target_idx));
        }
    }

    None
}

/// Helper to match destination page reference to 0-based page index.
fn resolve_dest_page_index(dest_obj: &PdfObject, pages: &[ObjectId]) -> Option<usize> {
    let target_ref = match dest_obj {
        PdfObject::Array(arr) if !arr.is_empty() => arr[0].as_reference()?,
        PdfObject::Reference(id) => *id,
        _ => return None,
    };

    pages.iter().position(|&p| p == target_ref)
}
