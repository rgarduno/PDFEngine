//! Surgical flattening of visual annotations (Highlights, Underlines, Strikeouts, Stamps)
//! into permanent page content stream graphics and text.

use crate::annots::delete::delete_annotation;
use crate::annots::reader::extract_page_annotations;
use crate::annots::types::{Annotation, AnnotationSubtype};
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};

/// Flattens all visual annotations (highlights, underlines, strikeouts, stamps)
/// on a specific page (or across the entire document if page_index is None) into permanent vectors and text.
///
/// Returns the number of annotations successfully flattened.
pub fn flatten_annotations(
    doc: &mut PdfDocument,
    page_index: Option<usize>,
) -> PdfResult<usize> {
    let pages = doc.get_pages()?;
    let targets: Vec<usize> = match page_index {
        Some(idx) => {
            if idx >= pages.len() {
                return Err(PdfError::InvalidPageNumber {
                    page: idx + 1,
                    total: pages.len(),
                });
            }
            vec![idx]
        }
        None => (0..pages.len()).collect(),
    };

    let mut total_flattened = 0;

    for idx in targets {
        let count = flatten_single_page_annotations(doc, idx)?;
        total_flattened += count;
    }

    Ok(total_flattened)
}

/// Flattens annotations on a single page.
fn flatten_single_page_annotations(doc: &mut PdfDocument, page_index: usize) -> PdfResult<usize> {
    let annots = extract_page_annotations(doc, page_index)?;
    let visual_annots: Vec<Annotation> = annots
        .into_iter()
        .filter(|a| matches!(
            a.subtype,
            AnnotationSubtype::Highlight
                | AnnotationSubtype::Underline
                | AnnotationSubtype::StrikeOut
                | AnnotationSubtype::Stamp
        ))
        .collect();

    if visual_annots.is_empty() {
        return Ok(0);
    }

    let pages = doc.get_pages()?;
    let page_id = pages[page_index];

    let mut append_ops = String::from("\n% --- Flattened Annotations ---\n");
    let mut needs_extg = false;
    let mut needs_stamp_font = false;

    for annot in &visual_annots {
        let rect = annot.rect;
        let width = rect.width().max(1.0);
        let height = rect.height().max(1.0);
        let c = annot.color.unwrap_or([1.0, 0.92, 0.23]);

        match annot.subtype {
            AnnotationSubtype::Highlight => {
                needs_extg = true;
                append_ops.push_str("q\n");
                append_ops.push_str("/GS_HL gs\n");
                append_ops.push_str(&format!("{:.3} {:.3} {:.3} rg\n", c[0], c[1], c[2]));
                append_ops.push_str(&format!(
                    "{:.2} {:.2} {:.2} {:.2} re f\n",
                    rect.min_x, rect.min_y, width, height
                ));
                append_ops.push_str("Q\n");
            }
            AnnotationSubtype::Underline => {
                append_ops.push_str("q\n");
                append_ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n", c[0], c[1], c[2]));
                append_ops.push_str("1.5 w\n");
                append_ops.push_str(&format!(
                    "{:.2} {:.2} m {:.2} {:.2} l S\n",
                    rect.min_x,
                    rect.min_y + 1.0,
                    rect.max_x,
                    rect.min_y + 1.0
                ));
                append_ops.push_str("Q\n");
            }
            AnnotationSubtype::StrikeOut => {
                let y_mid = rect.min_y + (height / 2.0);
                append_ops.push_str("q\n");
                append_ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n", c[0], c[1], c[2]));
                append_ops.push_str("1.5 w\n");
                append_ops.push_str(&format!(
                    "{:.2} {:.2} m {:.2} {:.2} l S\n",
                    rect.min_x, y_mid, rect.max_x, y_mid
                ));
                append_ops.push_str("Q\n");
            }
            AnnotationSubtype::Stamp => {
                needs_extg = true;
                needs_stamp_font = true;
                let text = annot
                    .contents
                    .clone()
                    .or_else(|| annot.stamp_type.as_ref().map(|s| s.text()))
                    .unwrap_or_else(|| "APPROVED".to_string());

                append_ops.push_str("q\n");
                append_ops.push_str("/GS_Stamp gs\n");
                append_ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n", c[0], c[1], c[2]));
                append_ops.push_str(&format!("{:.3} {:.3} {:.3} rg\n", c[0], c[1], c[2]));

                // Outer border
                let m = 2.0;
                append_ops.push_str("2.5 w\n");
                append_ops.push_str(&format!(
                    "{:.2} {:.2} {:.2} {:.2} re S\n",
                    rect.min_x + m,
                    rect.min_y + m,
                    width - (m * 2.0),
                    height - (m * 2.0)
                ));

                // Inner border
                let im = 5.0;
                append_ops.push_str("0.8 w\n");
                append_ops.push_str(&format!(
                    "{:.2} {:.2} {:.2} {:.2} re S\n",
                    rect.min_x + im,
                    rect.min_y + im,
                    width - (im * 2.0),
                    height - (im * 2.0)
                ));

                // Centered text
                let char_count = text.len().max(1) as f64;
                let font_size = (height * 0.45).min((width * 0.8) / (char_count * 0.65)).clamp(9.0, 24.0);
                let est_width = char_count * font_size * 0.58;
                let text_x = rect.min_x + ((width - est_width) / 2.0).max(4.0);
                let text_y = rect.min_y + (height - font_size) / 2.0 + (font_size * 0.22);

                append_ops.push_str("BT\n");
                append_ops.push_str(&format!("/F_Stamp {:.1} Tf\n", font_size));
                append_ops.push_str(&format!("{:.2} {:.2} Td\n", text_x, text_y));
                append_ops.push_str(&format!("({}) Tj\n", escape_pdf(&text)));
                append_ops.push_str("ET\n");
                append_ops.push_str("Q\n");
            }
            _ => {}
        }
    }

    // 1. Append operations to page /Contents stream
    append_ops_to_page_contents(doc, page_id, &append_ops)?;

    // 2. Ensure resources are declared on page
    ensure_annotation_page_resources(doc, page_id, needs_extg, needs_stamp_font)?;

    // 3. Purge flattened annotations from page /Annots and document objects
    let count = visual_annots.len();
    for annot in visual_annots {
        let _ = delete_annotation(doc, page_index, annot.id);
    }

    Ok(count)
}

/// Injects graphics operations into the page's /Contents stream.
fn append_ops_to_page_contents(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    ops: &str,
) -> PdfResult<()> {
    let mut existing_bytes = doc.get_page_content_bytes(page_id).unwrap_or_default();
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

/// Ensures required ExtGState and Font definitions are present in page /Resources.
fn ensure_annotation_page_resources(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    needs_extg: bool,
    needs_stamp_font: bool,
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

    // 1. ExtGState resources
    if needs_extg {
        let mut extg_dict = match res_dict.remove("ExtGState") {
            Some(PdfObject::Dictionary(d)) => d,
            _ => PdfDictionary::new(),
        };

        if !extg_dict.contains_key("GS_HL") {
            let mut gs_hl = PdfDictionary::new();
            gs_hl.insert("Type", PdfObject::Name("ExtGState".into()));
            gs_hl.insert("ca", PdfObject::Real(0.45));
            gs_hl.insert("CA", PdfObject::Real(0.45));
            gs_hl.insert("BM", PdfObject::Name("Multiply".into()));
            extg_dict.insert("GS_HL", PdfObject::Dictionary(gs_hl));
        }

        if !extg_dict.contains_key("GS_Stamp") {
            let mut gs_st = PdfDictionary::new();
            gs_st.insert("Type", PdfObject::Name("ExtGState".into()));
            gs_st.insert("ca", PdfObject::Real(0.85));
            gs_st.insert("CA", PdfObject::Real(0.85));
            extg_dict.insert("GS_Stamp", PdfObject::Dictionary(gs_st));
        }

        res_dict.insert("ExtGState", PdfObject::Dictionary(extg_dict));
    }

    // 2. Font resources
    if needs_stamp_font {
        let mut font_dict = match res_dict.remove("Font") {
            Some(PdfObject::Dictionary(d)) => d,
            _ => PdfDictionary::new(),
        };

        if !font_dict.contains_key("F_Stamp") {
            let mut f_dict = PdfDictionary::new();
            f_dict.insert("Type", PdfObject::Name("Font".into()));
            f_dict.insert("Subtype", PdfObject::Name("Type1".into()));
            f_dict.insert("BaseFont", PdfObject::Name("Helvetica-Bold".into()));
            font_dict.insert("F_Stamp", PdfObject::Dictionary(f_dict));
        }

        res_dict.insert("Font", PdfObject::Dictionary(font_dict));
    }

    page_dict.insert("Resources", PdfObject::Dictionary(res_dict));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(())
}

fn escape_pdf(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out
}
