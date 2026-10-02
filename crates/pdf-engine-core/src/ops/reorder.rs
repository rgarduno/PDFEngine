//! Page reordering and deletion operations according to ISO 32000-1.

use crate::cos::{ObjectId, PdfArray, PdfDocument, PdfObject};
use crate::error::{PdfError, PdfResult};

/// Deletes specified pages by index (0-based) from the document.
pub fn delete_pages(doc: &mut PdfDocument, page_indices: &[usize]) -> PdfResult<()> {
    let all_pages = doc.get_pages()?;
    let total = all_pages.len();

    let delete_set: std::collections::HashSet<usize> = page_indices.iter().copied().collect();

    for &idx in &delete_set {
        if idx >= total {
            return Err(PdfError::InvalidPageNumber {
                page: idx + 1,
                total,
            });
        }
    }

    if delete_set.len() >= total {
        return Err(PdfError::OperationError(
            "Cannot delete all pages from a document: at least one page must remain".to_string(),
        ));
    }

    let kept_page_ids: Vec<ObjectId> = all_pages
        .iter()
        .enumerate()
        .filter_map(|(i, &id)| if delete_set.contains(&i) { None } else { Some(id) })
        .collect();

    let pages_id = doc.pages_id()?;
    let mut pages_dict = match doc.get_object(pages_id)? {
        PdfObject::Dictionary(d) => d,
        _ => return Err(PdfError::OperationError("Root /Pages is not a dictionary".to_string())),
    };

    let kids_array: PdfArray = kept_page_ids
        .iter()
        .map(|&id| PdfObject::Reference(id))
        .collect();
    pages_dict.insert("Kids", PdfObject::Array(kids_array));
    pages_dict.insert("Count", kept_page_ids.len() as i64);

    doc.set_object(pages_id, PdfObject::Dictionary(pages_dict));

    Ok(())
}

/// Reorders the document's pages according to a new permutation of 0-based indices.
/// `new_order` must be a permutation of `0..doc.get_pages()?.len()`.
pub fn reorder_pages(doc: &mut PdfDocument, new_order: &[usize]) -> PdfResult<()> {
    let all_pages = doc.get_pages()?;
    let total = all_pages.len();

    if new_order.len() != total {
        return Err(PdfError::OperationError(format!(
            "new_order length ({}) does not match document page count ({})",
            new_order.len(),
            total
        )));
    }

    let mut seen = std::collections::HashSet::new();
    let mut reordered_ids = Vec::with_capacity(total);

    for &idx in new_order {
        if idx >= total {
            return Err(PdfError::InvalidPageNumber {
                page: idx + 1,
                total,
            });
        }
        if !seen.insert(idx) {
            return Err(PdfError::OperationError(format!(
                "Duplicate page index {} in new_order",
                idx
            )));
        }
        reordered_ids.push(all_pages[idx]);
    }

    let pages_id = doc.pages_id()?;
    let mut pages_dict = match doc.get_object(pages_id)? {
        PdfObject::Dictionary(d) => d,
        _ => return Err(PdfError::OperationError("Root /Pages is not a dictionary".to_string())),
    };

    let kids_array: PdfArray = reordered_ids
        .iter()
        .map(|&id| PdfObject::Reference(id))
        .collect();
    pages_dict.insert("Kids", PdfObject::Array(kids_array));
    pages_dict.insert("Count", total as i64);

    doc.set_object(pages_id, PdfObject::Dictionary(pages_dict));

    Ok(())
}
