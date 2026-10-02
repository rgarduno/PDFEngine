//! Document splitting and page extraction operations according to ISO 32000-1.

use crate::cos::PdfDocument;
use crate::error::{PdfError, PdfResult};
use crate::ops::cloner::ObjectCloner;

/// Extracts a specified subset of pages by index (0-based) into a new, self-contained `PdfDocument`.
pub fn extract_pages(
    doc: &mut PdfDocument,
    page_indices: &[usize],
) -> PdfResult<PdfDocument> {
    let all_pages = doc.get_pages()?;
    for &idx in page_indices {
        if idx >= all_pages.len() {
            return Err(PdfError::InvalidPageNumber {
                page: idx + 1,
                total: all_pages.len(),
            });
        }
    }

    let mut dest_doc = PdfDocument::empty();
    let mut cloner = ObjectCloner::new(doc)?;

    for &idx in page_indices {
        let src_page_id = all_pages[idx];
        cloner.import_page(doc, src_page_id, &mut dest_doc)?;
    }

    Ok(dest_doc)
}

/// Splits a document into chunks of `chunk_size` pages each.
/// For example, a 10-page document with chunk_size 1 produces 10 single-page documents.
pub fn split_document(
    doc: &mut PdfDocument,
    chunk_size: usize,
) -> PdfResult<Vec<PdfDocument>> {
    if chunk_size == 0 {
        return Err(PdfError::OperationError(
            "chunk_size must be at least 1".to_string(),
        ));
    }

    let all_pages = doc.get_pages()?;
    let total = all_pages.len();
    let mut result = Vec::new();

    let mut start = 0;
    while start < total {
        let end = (start + chunk_size).min(total);
        let indices: Vec<usize> = (start..end).collect();
        result.push(extract_pages(doc, &indices)?);
        start = end;
    }

    Ok(result)
}

/// Splits a document by explicit page index ranges (e.g. `[(0, 2), (3, 5)]` 0-indexed inclusive).
pub fn split_by_ranges(
    doc: &mut PdfDocument,
    ranges: &[(usize, usize)],
) -> PdfResult<Vec<PdfDocument>> {
    let mut result = Vec::new();
    for &(start, end) in ranges {
        if start > end {
            return Err(PdfError::OperationError(format!(
                "Invalid range: start {} is greater than end {}",
                start, end
            )));
        }
        let indices: Vec<usize> = (start..=end).collect();
        result.push(extract_pages(doc, &indices)?);
    }
    Ok(result)
}
