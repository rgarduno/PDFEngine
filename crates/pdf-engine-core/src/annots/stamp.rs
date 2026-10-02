//! Rubber Stamp annotations according to ISO 32000-1 §12.5.6.12.

use crate::annots::markup::attach_annotation_to_page;
use crate::annots::types::StampType;
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream, PdfString};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::layout::geometry::Rect;

/// Adds a styled vector rubber stamp annotation to a page.
pub fn add_stamp(
    doc: &mut PdfDocument,
    page_index: usize,
    stamp_type: StampType,
    rect: Option<Rect>,
    custom_text: Option<&str>,
    color: Option<[f64; 3]>,
    date_str: Option<&str>,
) -> PdfResult<ObjectId> {
    let pages = doc.get_pages()?;
    let page_id = pages.get(page_index).copied().ok_or_else(|| {
        PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        }
    })?;

    // Default dimensions: 160 x 50 pt located in upper region of page
    let final_rect = rect.unwrap_or_else(|| Rect::new(400.0, 700.0, 560.0, 750.0));
    let text = custom_text
        .map(|s| s.to_string())
        .unwrap_or_else(|| stamp_type.text());
    let c = color.unwrap_or_else(|| stamp_type.default_color());

    let width = final_rect.width().max(60.0);
    let height = final_rect.height().max(25.0);

    // 1. Synthesize crisp vector rubber stamp appearance stream (/AP /N)
    let stream_bytes = build_stamp_appearance_stream(&text, date_str, width, height, c);

    let mut ap_dict = PdfDictionary::new();
    ap_dict.insert("Type", PdfObject::Name("XObject".into()));
    ap_dict.insert("Subtype", PdfObject::Name("Form".into()));
    ap_dict.insert(
        "BBox",
        PdfObject::Array(vec![
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
            PdfObject::Real(width),
            PdfObject::Real(height),
        ]),
    );

    // Resources with standard Helvetica-Bold font and ExtGState opacity
    let mut font_dict = PdfDictionary::new();
    let mut f1_dict = PdfDictionary::new();
    f1_dict.insert("Type", PdfObject::Name("Font".into()));
    f1_dict.insert("Subtype", PdfObject::Name("Type1".into()));
    f1_dict.insert("BaseFont", PdfObject::Name("Helvetica-Bold".into()));
    font_dict.insert("F1", PdfObject::Dictionary(f1_dict));

    let mut gs_dict = PdfDictionary::new();
    gs_dict.insert("Type", PdfObject::Name("ExtGState".into()));
    gs_dict.insert("CA", PdfObject::Real(0.88));
    gs_dict.insert("ca", PdfObject::Real(0.88));

    let mut extg_dict = PdfDictionary::new();
    extg_dict.insert("GS0", PdfObject::Dictionary(gs_dict));

    let mut res_dict = PdfDictionary::new();
    res_dict.insert("Font", PdfObject::Dictionary(font_dict));
    res_dict.insert("ExtGState", PdfObject::Dictionary(extg_dict));
    ap_dict.insert("Resources", PdfObject::Dictionary(res_dict));

    let ap_stream = PdfStream::new(ap_dict, stream_bytes);
    let ap_stream_id = doc.alloc_object_id();
    doc.set_object(ap_stream_id, PdfObject::Stream(ap_stream));

    // 2. Build Stamp Annotation Dictionary
    let mut annot_dict = PdfDictionary::new();
    annot_dict.insert("Type", PdfObject::Name("Annot".into()));
    annot_dict.insert("Subtype", PdfObject::Name("Stamp".into()));
    annot_dict.insert(
        "Rect",
        PdfObject::Array(vec![
            PdfObject::Real(final_rect.min_x),
            PdfObject::Real(final_rect.min_y),
            PdfObject::Real(final_rect.max_x),
            PdfObject::Real(final_rect.max_y),
        ]),
    );
    annot_dict.insert("Name", PdfObject::Name(stamp_type.text().as_str().into()));
    annot_dict.insert(
        "Contents",
        PdfObject::String(PdfString::literal(text.as_bytes())),
    );
    annot_dict.insert(
        "C",
        PdfObject::Array(vec![
            PdfObject::Real(c[0]),
            PdfObject::Real(c[1]),
            PdfObject::Real(c[2]),
        ]),
    );
    annot_dict.insert("F", PdfObject::Integer(4)); // Print
    annot_dict.insert("P", PdfObject::Reference(page_id));

    if let Some(d) = date_str {
        annot_dict.insert("M", PdfObject::String(PdfString::literal(d.as_bytes())));
    }

    let mut ap_sub_dict = PdfDictionary::new();
    ap_sub_dict.insert("N", PdfObject::Reference(ap_stream_id));
    annot_dict.insert("AP", PdfObject::Dictionary(ap_sub_dict));

    let annot_id = doc.alloc_object_id();
    doc.set_object(annot_id, PdfObject::Dictionary(annot_dict));

    attach_annotation_to_page(doc, page_id, annot_id)?;

    Ok(annot_id)
}

/// Generates vector graphics operations for rubber stamp appearance.
fn build_stamp_appearance_stream(
    text: &str,
    date_str: Option<&str>,
    width: f64,
    height: f64,
    color: [f64; 3],
) -> Vec<u8> {
    let mut ops = String::new();
    ops.push_str("q\n");
    ops.push_str("/GS0 gs\n");

    // Colors
    ops.push_str(&format!(
        "{:.3} {:.3} {:.3} RG\n",
        color[0], color[1], color[2]
    ));
    ops.push_str(&format!(
        "{:.3} {:.3} {:.3} rg\n",
        color[0], color[1], color[2]
    ));

    // Outer thick rounded border (2.5 pt)
    let margin = 2.0;
    let b_w = width - (margin * 2.0);
    let b_h = height - (margin * 2.0);
    ops.push_str("2.5 w\n");
    ops.push_str(&format!(
        "{:.2} {:.2} {:.2} {:.2} re S\n",
        margin, margin, b_w, b_h
    ));

    // Inner subtle border (0.8 pt)
    let inner_margin = 5.0;
    let in_w = width - (inner_margin * 2.0);
    let in_h = height - (inner_margin * 2.0);
    ops.push_str("0.8 w\n");
    ops.push_str(&format!(
        "{:.2} {:.2} {:.2} {:.2} re S\n",
        inner_margin, inner_margin, in_w, in_h
    ));

    // Text Font Sizing & Centering
    let char_count = text.len().max(1) as f64;
    let main_font_size = if date_str.is_some() {
        (height * 0.42).min((width * 0.85) / (char_count * 0.65)).clamp(8.0, 22.0)
    } else {
        (height * 0.52).min((width * 0.85) / (char_count * 0.65)).clamp(10.0, 26.0)
    };

    let est_text_width = char_count * main_font_size * 0.58;
    let text_x = ((width - est_text_width) / 2.0).max(6.0);
    let text_y = if date_str.is_some() {
        height * 0.52
    } else {
        (height - main_font_size) / 2.0 + (main_font_size * 0.25)
    };

    ops.push_str("BT\n");
    ops.push_str(&format!("/F1 {:.1} Tf\n", main_font_size));
    ops.push_str(&format!("{:.2} {:.2} Td\n", text_x, text_y));
    ops.push_str(&format!("({}) Tj\n", escape_pdf_text(text)));
    ops.push_str("ET\n");

    // Optional Date Subtitle
    if let Some(date) = date_str {
        let date_font_size = (main_font_size * 0.52).clamp(6.0, 11.0);
        let date_char_count = date.len().max(1) as f64;
        let est_date_width = date_char_count * date_font_size * 0.55;
        let date_x = ((width - est_date_width) / 2.0).max(6.0);
        let date_y = height * 0.22;

        ops.push_str("BT\n");
        ops.push_str(&format!("/F1 {:.1} Tf\n", date_font_size));
        ops.push_str(&format!("{:.2} {:.2} Td\n", date_x, date_y));
        ops.push_str(&format!("({}) Tj\n", escape_pdf_text(date)));
        ops.push_str("ET\n");
    }

    ops.push_str("Q\n");
    ops.into_bytes()
}

/// Escapes parentheses and backslashes for PDF literal strings.
fn escape_pdf_text(s: &str) -> String {
    let mut res = String::new();
    for c in s.chars() {
        match c {
            '(' => res.push_str("\\("),
            ')' => res.push_str("\\)"),
            '\\' => res.push_str("\\\\"),
            other => res.push(other),
        }
    }
    res
}
