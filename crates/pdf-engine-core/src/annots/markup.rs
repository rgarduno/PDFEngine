//! Text markup annotations (Highlight, Underline, StrikeOut) according to ISO 32000-1 §12.5.6.10.

use crate::annots::types::AnnotationSubtype;
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream, PdfString};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::layout::geometry::Rect;

/// Adds a text markup annotation (Highlight, Underline, StrikeOut) to a specific page.
///
/// Automatically generates ISO 32000 QuadPoints and a synthesized `/AP /N` vector Form XObject
/// ensuring pixel-perfect rendering across all PDF viewing software.
pub fn add_text_markup(
    doc: &mut PdfDocument,
    page_index: usize,
    subtype: AnnotationSubtype,
    rect: Rect,
    quad_points: Option<&[f64]>,
    color: Option<[f64; 3]>,
    opacity: Option<f64>,
    contents: Option<&str>,
) -> PdfResult<ObjectId> {
    let pages = doc.get_pages()?;
    let page_id = pages.get(page_index).copied().ok_or_else(|| {
        PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        }
    })?;

    // Determine colors and opacities according to subtype defaults
    let (default_color, default_opacity) = match subtype {
        AnnotationSubtype::Highlight => ([1.0, 0.92, 0.23], 0.45), // Classic highlighter yellow
        AnnotationSubtype::Underline => ([0.15, 0.45, 0.90], 1.0),  // Crisp blue
        AnnotationSubtype::StrikeOut => ([0.85, 0.15, 0.15], 1.0),  // Clear red
        _ => ([1.0, 1.0, 0.0], 0.5),
    };

    let c = color.unwrap_or(default_color);
    let alpha = opacity.unwrap_or(default_opacity).clamp(0.0, 1.0);

    // QuadPoints: 8 numbers per quad (x1 y1 x2 y2 x3 y3 x4 y4 = top-left, top-right, bottom-left, bottom-right)
    let quads = if let Some(qp) = quad_points {
        if qp.len() >= 8 {
            qp.to_vec()
        } else {
            generate_default_quad_points(&rect)
        }
    } else {
        generate_default_quad_points(&rect)
    };

    let width = rect.width().max(1.0);
    let height = rect.height().max(1.0);

    // 1. Synthesize Appearance Stream (/AP /N)
    let ap_stream_bytes = build_markup_appearance_stream(subtype, width, height, c, alpha);
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

    // ExtGState for transparency if alpha < 1.0
    if alpha < 0.999 {
        let mut gs_dict = PdfDictionary::new();
        gs_dict.insert("Type", PdfObject::Name("ExtGState".into()));
        gs_dict.insert("ca", PdfObject::Real(alpha));
        gs_dict.insert("CA", PdfObject::Real(alpha));
        if subtype == AnnotationSubtype::Highlight {
            gs_dict.insert("BM", PdfObject::Name("Multiply".into()));
        }

        let mut extg_dict = PdfDictionary::new();
        extg_dict.insert("GS0", PdfObject::Dictionary(gs_dict));

        let mut res_dict = PdfDictionary::new();
        res_dict.insert("ExtGState", PdfObject::Dictionary(extg_dict));
        ap_dict.insert("Resources", PdfObject::Dictionary(res_dict));
    } else {
        ap_dict.insert("Resources", PdfObject::Dictionary(PdfDictionary::new()));
    }

    let ap_stream = PdfStream::new(ap_dict, ap_stream_bytes);
    let ap_stream_id = doc.alloc_object_id();
    doc.set_object(ap_stream_id, PdfObject::Stream(ap_stream));

    // 2. Build Annotation Dictionary
    let mut annot_dict = PdfDictionary::new();
    annot_dict.insert("Type", PdfObject::Name("Annot".into()));
    annot_dict.insert("Subtype", PdfObject::Name(subtype.as_pdf_name().into()));
    annot_dict.insert(
        "Rect",
        PdfObject::Array(vec![
            PdfObject::Real(rect.min_x),
            PdfObject::Real(rect.min_y),
            PdfObject::Real(rect.max_x),
            PdfObject::Real(rect.max_y),
        ]),
    );
    annot_dict.insert(
        "QuadPoints",
        PdfObject::Array(quads.into_iter().map(PdfObject::Real).collect()),
    );
    annot_dict.insert(
        "C",
        PdfObject::Array(vec![
            PdfObject::Real(c[0]),
            PdfObject::Real(c[1]),
            PdfObject::Real(c[2]),
        ]),
    );
    annot_dict.insert("CA", PdfObject::Real(alpha));
    annot_dict.insert("F", PdfObject::Integer(4)); // Print flag
    annot_dict.insert("P", PdfObject::Reference(page_id));

    if let Some(txt) = contents {
        annot_dict.insert("Contents", PdfObject::String(PdfString::literal(txt.as_bytes())));
    }

    let mut ap_sub_dict = PdfDictionary::new();
    ap_sub_dict.insert("N", PdfObject::Reference(ap_stream_id));
    annot_dict.insert("AP", PdfObject::Dictionary(ap_sub_dict));

    let annot_id = doc.alloc_object_id();
    doc.set_object(annot_id, PdfObject::Dictionary(annot_dict));

    // 3. Attach annotation to page /Annots array
    attach_annotation_to_page(doc, page_id, annot_id)?;

    Ok(annot_id)
}

/// Helper generating standard 8-point QuadPoints array from a bounding box.
fn generate_default_quad_points(rect: &Rect) -> Vec<f64> {
    vec![
        rect.min_x, rect.max_y, // Top-Left
        rect.max_x, rect.max_y, // Top-Right
        rect.min_x, rect.min_y, // Bottom-Left
        rect.max_x, rect.min_y, // Bottom-Right
    ]
}

/// Synthesizes pure vector ISO 32000 graphics operations for markup appearance stream.
fn build_markup_appearance_stream(
    subtype: AnnotationSubtype,
    width: f64,
    height: f64,
    color: [f64; 3],
    alpha: f64,
) -> Vec<u8> {
    let mut ops = String::new();
    ops.push_str("q\n");

    if alpha < 0.999 {
        ops.push_str("/GS0 gs\n");
    }

    match subtype {
        AnnotationSubtype::Highlight => {
            // Filled rectangle
            ops.push_str(&format!("{:.3} {:.3} {:.3} rg\n", color[0], color[1], color[2]));
            ops.push_str(&format!("0 0 {:.2} {:.2} re\n", width, height));
            ops.push_str("f\n");
        }
        AnnotationSubtype::Underline => {
            // Line along bottom edge
            ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n", color[0], color[1], color[2]));
            ops.push_str("1.5 w\n");
            ops.push_str(&format!("0 1 m {:.2} 1 l S\n", width));
        }
        AnnotationSubtype::StrikeOut => {
            // Line centered vertically
            let y_mid = height / 2.0;
            ops.push_str(&format!("{:.3} {:.3} {:.3} RG\n", color[0], color[1], color[2]));
            ops.push_str("1.5 w\n");
            ops.push_str(&format!("0 {:.2} m {:.2} {:.2} l S\n", y_mid, width, y_mid));
        }
        _ => {}
    }

    ops.push_str("Q\n");
    ops.into_bytes()
}

/// Helper function to attach an annotation ObjectId to a page's /Annots array.
pub(crate) fn attach_annotation_to_page(
    doc: &mut PdfDocument,
    page_id: ObjectId,
    annot_id: ObjectId,
) -> PdfResult<()> {
    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => {
            return Err(PdfError::OperationError(
                "Target page object is not a dictionary".into(),
            ))
        }
    };

    let mut annots = match page_dict.remove("Annots") {
        Some(PdfObject::Array(arr)) => arr,
        Some(PdfObject::Reference(ref_id)) => match doc.get_object(ref_id)? {
            PdfObject::Array(arr) => {
                let mut new_arr = arr;
                new_arr.push(PdfObject::Reference(annot_id));
                doc.set_object(ref_id, PdfObject::Array(new_arr));
                page_dict.insert("Annots", PdfObject::Reference(ref_id));
                doc.set_object(page_id, PdfObject::Dictionary(page_dict));
                return Ok(());
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };

    annots.push(PdfObject::Reference(annot_id));
    page_dict.insert("Annots", PdfObject::Array(annots));
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(())
}
