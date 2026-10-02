//! Document concatenation and merging operations according to ISO 32000-1.

use crate::cos::PdfDocument;
use crate::error::PdfResult;
use crate::ops::cloner::ObjectCloner;

/// Merges multiple `PdfDocument` instances into a single combined document.
/// All pages from each document are appended in order. Shared resources within each
/// source document remain deduplicated.
pub fn merge_documents(docs: &mut [PdfDocument]) -> PdfResult<PdfDocument> {
    let mut merged = PdfDocument::empty();

    for src_doc in docs {
        let pages = src_doc.get_pages()?;
        let mut cloner = ObjectCloner::new(src_doc)?;
        for page_id in pages {
            cloner.import_page(src_doc, page_id, &mut merged)?;
        }
    }

    Ok(merged)
}

/// Commercial convenience function: merges raw PDF byte buffers and returns serialized bytes.
pub fn merge_pdf_bytes(pdf_buffers: &[&[u8]]) -> PdfResult<Vec<u8>> {
    let mut parsed_docs = Vec::with_capacity(pdf_buffers.len());
    for buf in pdf_buffers {
        parsed_docs.push(PdfDocument::load(buf)?);
    }

    let mut merged = merge_documents(&mut parsed_docs)?;
    merged.save_to_vec()
}
