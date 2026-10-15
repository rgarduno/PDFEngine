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
        src_doc.require_copy()?;
        import_pages(src_doc, &mut merged)?;
    }

    Ok(merged)
}

/// Appends every page of `source` after every page of `dest`.
///
/// `dest` may assemble without being allowed to copy. `source` must be allowed
/// to copy. The result is a new document and does not keep a session gate.
pub fn append_document(dest: &mut PdfDocument, source: &mut PdfDocument) -> PdfResult<PdfDocument> {
    dest.require_assemble()?;
    source.require_copy()?;
    let mut merged = PdfDocument::empty();
    import_pages(dest, &mut merged)?;
    import_pages(source, &mut merged)?;
    Ok(merged)
}

fn import_pages(src_doc: &mut PdfDocument, merged: &mut PdfDocument) -> PdfResult<()> {
    let pages = src_doc.get_pages()?;
    let mut cloner = ObjectCloner::new(src_doc)?;
    for page_id in pages {
        cloner.import_page(src_doc, page_id, merged)?;
    }
    Ok(())
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
