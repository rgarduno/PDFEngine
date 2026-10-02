//! Interactive Link annotations (Web URIs and Internal GoTo destinations) according to ISO 32000-1 §12.5.6.5.

use crate::annots::markup::attach_annotation_to_page;
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfString};
use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::layout::geometry::Rect;

/// Adds an interactive clickable web link annotation to a page.
pub fn add_link_uri(
    doc: &mut PdfDocument,
    page_index: usize,
    rect: Rect,
    uri: &str,
    show_border: bool,
) -> PdfResult<ObjectId> {
    let pages = doc.get_pages()?;
    let page_id = pages.get(page_index).copied().ok_or_else(|| {
        PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        }
    })?;

    let mut action_dict = PdfDictionary::new();
    action_dict.insert("S", PdfObject::Name("URI".into()));
    action_dict.insert("URI", PdfObject::String(PdfString::literal(uri.as_bytes())));

    let mut annot_dict = PdfDictionary::new();
    annot_dict.insert("Type", PdfObject::Name("Annot".into()));
    annot_dict.insert("Subtype", PdfObject::Name("Link".into()));
    annot_dict.insert(
        "Rect",
        PdfObject::Array(vec![
            PdfObject::Real(rect.min_x),
            PdfObject::Real(rect.min_y),
            PdfObject::Real(rect.max_x),
            PdfObject::Real(rect.max_y),
        ]),
    );
    annot_dict.insert("A", PdfObject::Dictionary(action_dict));
    annot_dict.insert("H", PdfObject::Name("I".into())); // Invert on click
    annot_dict.insert("F", PdfObject::Integer(4)); // Print flag
    annot_dict.insert("P", PdfObject::Reference(page_id));

    if show_border {
        // Subtle blue underline border
        annot_dict.insert(
            "C",
            PdfObject::Array(vec![
                PdfObject::Real(0.1),
                PdfObject::Real(0.3),
                PdfObject::Real(0.85),
            ]),
        );
        let mut bs_dict = PdfDictionary::new();
        bs_dict.insert("W", PdfObject::Real(1.0));
        bs_dict.insert("S", PdfObject::Name("U".into())); // Underline style
        annot_dict.insert("BS", PdfObject::Dictionary(bs_dict));
    } else {
        annot_dict.insert(
            "Border",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
            ]),
        );
    }

    let annot_id = doc.alloc_object_id();
    doc.set_object(annot_id, PdfObject::Dictionary(annot_dict));

    attach_annotation_to_page(doc, page_id, annot_id)?;

    Ok(annot_id)
}

/// Adds an internal document jump link annotation targeting another page.
pub fn add_link_goto(
    doc: &mut PdfDocument,
    page_index: usize,
    rect: Rect,
    target_page_index: usize,
) -> PdfResult<ObjectId> {
    let pages = doc.get_pages()?;
    let page_id = pages.get(page_index).copied().ok_or_else(|| {
        PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        }
    })?;

    let target_page_id = pages.get(target_page_index).copied().ok_or_else(|| {
        PdfError::InvalidPageNumber {
            page: target_page_index + 1,
            total: pages.len(),
        }
    })?;

    let mut action_dict = PdfDictionary::new();
    action_dict.insert("S", PdfObject::Name("GoTo".into()));
    // [page_ref /XYZ null null null] (retains current scroll/zoom level)
    action_dict.insert(
        "D",
        PdfObject::Array(vec![
            PdfObject::Reference(target_page_id),
            PdfObject::Name("XYZ".into()),
            PdfObject::Null,
            PdfObject::Null,
            PdfObject::Null,
        ]),
    );

    let mut annot_dict = PdfDictionary::new();
    annot_dict.insert("Type", PdfObject::Name("Annot".into()));
    annot_dict.insert("Subtype", PdfObject::Name("Link".into()));
    annot_dict.insert(
        "Rect",
        PdfObject::Array(vec![
            PdfObject::Real(rect.min_x),
            PdfObject::Real(rect.min_y),
            PdfObject::Real(rect.max_x),
            PdfObject::Real(rect.max_y),
        ]),
    );
    annot_dict.insert("A", PdfObject::Dictionary(action_dict));
    annot_dict.insert("H", PdfObject::Name("I".into()));
    annot_dict.insert("F", PdfObject::Integer(4));
    annot_dict.insert("P", PdfObject::Reference(page_id));
    annot_dict.insert(
        "Border",
        PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(0),
        ]),
    );

    let annot_id = doc.alloc_object_id();
    doc.set_object(annot_id, PdfObject::Dictionary(annot_dict));

    attach_annotation_to_page(doc, page_id, annot_id)?;

    Ok(annot_id)
}
