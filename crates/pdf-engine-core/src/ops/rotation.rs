//! Page rotation operations according to ISO 32000-1 §7.7.3.3.

use crate::cos::{PdfDocument, PdfObject};
use crate::error::{PdfError, PdfResult};

/// Normalizes an integer degree value to a canonical PDF rotation (0, 90, 180, or 270).
pub fn normalize_rotation(degrees: i32) -> i32 {
    let rem = ((degrees % 360) + 360) % 360;
    match rem {
        0..=44 | 315..=359 => 0,
        45..=134 => 90,
        135..=224 => 180,
        _ => 270,
    }
}

/// Retrieves the current rotation of a page in degrees (0, 90, 180, 270).
/// Resolves inheritance from the `/Pages` tree if not specified directly on the page dictionary.
pub fn get_page_rotation(doc: &mut PdfDocument, page_index: usize) -> PdfResult<i32> {
    let pages = doc.get_pages()?;
    let page_id = pages.get(page_index).copied().ok_or_else(|| {
        PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        }
    })?;

    let page_obj = doc.get_object(page_id)?;
    if let PdfObject::Dictionary(dict) = page_obj {
        if let Some(r) = dict.get("Rotate").and_then(|r| r.as_i64()) {
            return Ok(normalize_rotation(r as i32));
        }

        // Check inheritance from /Parent
        let mut curr_parent = dict.get("Parent").and_then(|p| p.as_reference());
        while let Some(parent_id) = curr_parent {
            if let Ok(PdfObject::Dictionary(parent_dict)) = doc.get_object(parent_id) {
                if let Some(r) = parent_dict.get("Rotate").and_then(|r| r.as_i64()) {
                    return Ok(normalize_rotation(r as i32));
                }
                curr_parent = parent_dict.get("Parent").and_then(|p| p.as_reference());
            } else {
                break;
            }
        }
    }

    Ok(0)
}

/// Sets the explicit rotation of a page in degrees (normalized to 0, 90, 180, 270).
pub fn set_page_rotation(
    doc: &mut PdfDocument,
    page_index: usize,
    rotation: i32,
) -> PdfResult<i32> {
    let canonical = normalize_rotation(rotation);
    let pages = doc.get_pages()?;
    let page_id = pages.get(page_index).copied().ok_or_else(|| {
        PdfError::InvalidPageNumber {
            page: page_index + 1,
            total: pages.len(),
        }
    })?;

    let page_obj = doc.get_object(page_id)?;
    let mut page_dict = match page_obj {
        PdfObject::Dictionary(d) => d,
        _ => {
            return Err(PdfError::TypeMismatch {
                id: page_id.number,
                gen: page_id.generation,
                expected: "Dictionary",
                found: "Non-dictionary page",
            })
        }
    };

    page_dict.insert("Rotate", canonical as i64);
    doc.set_object(page_id, PdfObject::Dictionary(page_dict));

    Ok(canonical)
}

/// Rotates a page relatively by adding `delta_degrees` (e.g. +90 or -90 clockwise).
pub fn rotate_page(
    doc: &mut PdfDocument,
    page_index: usize,
    delta_degrees: i32,
) -> PdfResult<i32> {
    let current = get_page_rotation(doc, page_index)?;
    let new_rotation = normalize_rotation(current + delta_degrees);
    set_page_rotation(doc, page_index, new_rotation)
}

/// Rotates all pages in the document by `delta_degrees`.
pub fn rotate_all_pages(doc: &mut PdfDocument, delta_degrees: i32) -> PdfResult<()> {
    let page_count = doc.get_pages()?.len();
    for i in 0..page_count {
        rotate_page(doc, i, delta_degrees)?;
    }
    Ok(())
}
