use crate::cos::{ObjectId, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, Writer};
use crate::ops::{
    delete_pages, extract_pages, get_page_rotation, merge_documents, merge_pdf_bytes,
    reorder_pages, rotate_all_pages, rotate_page, set_page_rotation, split_by_ranges,
    split_document,
};

fn create_test_multipage_pdf(page_texts: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut writer = Writer::new(&mut out);
    writer.write_header("1.7").unwrap();

    let mut offsets = Vec::new();
    let catalog_id = ObjectId::new(1);
    let pages_id = ObjectId::new(2);

    let num_pages = page_texts.len();
    let mut page_refs = Vec::new();

    let mut next_id = 3;
    let mut page_objs = Vec::new();
    let mut stream_objs = Vec::new();

    for text in page_texts {
        let page_id = ObjectId::new(next_id);
        next_id += 1;
        let content_id = ObjectId::new(next_id);
        next_id += 1;

        page_refs.push(PdfObject::Reference(page_id));

        let stream_bytes = format!("BT /F1 12 Tf 72 700 Td ({}) Tj ET", text).into_bytes();
        let mut stream_dict = PdfDictionary::new();
        stream_dict.insert("Length", stream_bytes.len() as i64);

        stream_objs.push((
            content_id,
            PdfObject::Stream(PdfStream {
                dict: stream_dict,
                content: stream_bytes,
            }),
        ));

        let mut page_dict = PdfDictionary::new();
        page_dict.insert("Type", PdfName::new("Page"));
        page_dict.insert("Parent", pages_id);
        page_dict.insert(
            "MediaBox",
            vec![0i64.into(), 0i64.into(), 612i64.into(), 792i64.into()],
        );
        page_dict.insert("Contents", content_id);

        let mut font_f1 = PdfDictionary::new();
        font_f1.insert("Type", PdfName::new("Font"));
        font_f1.insert("Subtype", PdfName::new("Type1"));
        font_f1.insert("BaseFont", PdfName::new("Helvetica"));

        let mut font_dict = PdfDictionary::new();
        font_dict.insert("F1", PdfObject::Dictionary(font_f1));

        let mut res_dict = PdfDictionary::new();
        res_dict.insert("Font", PdfObject::Dictionary(font_dict));
        page_dict.insert("Resources", PdfObject::Dictionary(res_dict));

        page_objs.push((page_id, PdfObject::Dictionary(page_dict)));
    }

    let mut catalog_dict = PdfDictionary::new();
    catalog_dict.insert("Type", PdfName::new("Catalog"));
    catalog_dict.insert("Pages", pages_id);
    let off = writer
        .write_indirect_object(catalog_id, &PdfObject::Dictionary(catalog_dict))
        .unwrap();
    offsets.push((catalog_id, off));

    let mut pages_dict = PdfDictionary::new();
    pages_dict.insert("Type", PdfName::new("Pages"));
    pages_dict.insert("Kids", page_refs);
    pages_dict.insert("Count", num_pages as i64);
    let off = writer
        .write_indirect_object(pages_id, &PdfObject::Dictionary(pages_dict))
        .unwrap();
    offsets.push((pages_id, off));

    for (id, obj) in page_objs {
        let off = writer.write_indirect_object(id, &obj).unwrap();
        offsets.push((id, off));
    }
    for (id, obj) in stream_objs {
        let off = writer.write_indirect_object(id, &obj).unwrap();
        offsets.push((id, off));
    }

    let mut trailer = PdfDictionary::new();
    trailer.insert("Root", catalog_id);
    trailer.insert("Size", (offsets.len() + 1) as i64);
    writer.write_xref_and_trailer(&offsets, &trailer).unwrap();

    out
}

#[test]
fn test_page_rotation() {
    let bytes = create_test_multipage_pdf(&["Page 1 Content", "Page 2 Content"]);
    let mut doc = PdfDocument::load(&bytes).expect("Failed to load PDF");

    // Initially 0 degrees
    assert_eq!(get_page_rotation(&mut doc, 0).unwrap(), 0);
    assert_eq!(get_page_rotation(&mut doc, 1).unwrap(), 0);

    // Rotate page 0 by 90
    assert_eq!(rotate_page(&mut doc, 0, 90).unwrap(), 90);
    assert_eq!(get_page_rotation(&mut doc, 0).unwrap(), 90);

    // Rotate page 0 relatively by another 90 (total 180)
    assert_eq!(rotate_page(&mut doc, 0, 90).unwrap(), 180);
    assert_eq!(get_page_rotation(&mut doc, 0).unwrap(), 180);

    // Set page 1 directly to 270
    assert_eq!(set_page_rotation(&mut doc, 1, 270).unwrap(), 270);
    assert_eq!(get_page_rotation(&mut doc, 1).unwrap(), 270);

    // Rotate all by 90
    rotate_all_pages(&mut doc, 90).unwrap();
    assert_eq!(get_page_rotation(&mut doc, 0).unwrap(), 270);
    assert_eq!(get_page_rotation(&mut doc, 1).unwrap(), 0);

    // Roundtrip test: serialize and reload
    let saved = doc.save_to_vec().expect("Failed to save PDF");
    let mut reloaded = PdfDocument::load(&saved).expect("Failed to reload PDF");
    assert_eq!(get_page_rotation(&mut reloaded, 0).unwrap(), 270);
    assert_eq!(get_page_rotation(&mut reloaded, 1).unwrap(), 0);
}

#[test]
fn test_extract_and_split_pages() {
    let bytes = create_test_multipage_pdf(&[
        "Page 0 Alpha",
        "Page 1 Beta",
        "Page 2 Gamma",
        "Page 3 Delta",
    ]);
    let mut doc = PdfDocument::load(&bytes).expect("Failed to load PDF");

    // Extract pages [1, 3] (Beta and Delta)
    let mut extracted = extract_pages(&mut doc, &[1, 3]).expect("Failed to extract pages");
    let extracted_pages = extracted.get_pages().expect("Failed to get pages");
    assert_eq!(extracted_pages.len(), 2);

    let content_0 = extracted
        .get_page_content_bytes(extracted_pages[0])
        .unwrap();
    let content_1 = extracted
        .get_page_content_bytes(extracted_pages[1])
        .unwrap();
    assert!(String::from_utf8_lossy(&content_0).contains("Beta"));
    assert!(String::from_utf8_lossy(&content_1).contains("Delta"));

    // Verify extracted document is completely self-contained and valid
    let saved_extracted = extracted.save_to_vec().expect("Failed to save extracted");
    let mut reloaded_extracted = PdfDocument::load(&saved_extracted).expect("Failed to reload");
    assert_eq!(reloaded_extracted.get_pages().unwrap().len(), 2);

    // Test split_document into chunks of 2
    let chunks = split_document(&mut doc, 2).expect("Failed to split document");
    assert_eq!(chunks.len(), 2);
    let mut c0 = chunks[0].clone();
    let mut c1 = chunks[1].clone();
    assert_eq!(c0.get_pages().unwrap().len(), 2);
    assert_eq!(c1.get_pages().unwrap().len(), 2);

    // Test split_by_ranges
    let range_docs = split_by_ranges(&mut doc, &[(0, 0), (1, 2)]).unwrap();
    assert_eq!(range_docs.len(), 2);
    let mut r0 = range_docs[0].clone();
    let mut r1 = range_docs[1].clone();
    assert_eq!(r0.get_pages().unwrap().len(), 1);
    assert_eq!(r1.get_pages().unwrap().len(), 2);
}

#[test]
fn test_merge_documents() {
    let bytes_a = create_test_multipage_pdf(&["Doc A - Page 1", "Doc A - Page 2"]);
    let bytes_b = create_test_multipage_pdf(&["Doc B - Page 1"]);

    let doc_a = PdfDocument::load(&bytes_a).unwrap();
    let doc_b = PdfDocument::load(&bytes_b).unwrap();

    let mut docs = [doc_a, doc_b];
    let mut merged = merge_documents(&mut docs).expect("Failed to merge");
    let pages = merged.get_pages().expect("Failed to get pages from merged");
    assert_eq!(pages.len(), 3);

    let p0 = merged.get_page_content_bytes(pages[0]).unwrap();
    let p1 = merged.get_page_content_bytes(pages[1]).unwrap();
    let p2 = merged.get_page_content_bytes(pages[2]).unwrap();

    assert!(String::from_utf8_lossy(&p0).contains("Doc A - Page 1"));
    assert!(String::from_utf8_lossy(&p1).contains("Doc A - Page 2"));
    assert!(String::from_utf8_lossy(&p2).contains("Doc B - Page 1"));

    // Verify merged document roundtrips
    let merged_bytes = merged.save_to_vec().expect("Failed to save merged");
    let mut reloaded = PdfDocument::load(&merged_bytes).expect("Failed to reload merged");
    assert_eq!(reloaded.get_pages().unwrap().len(), 3);

    // Test convenience function merge_pdf_bytes
    let fast_merged_bytes = merge_pdf_bytes(&[&bytes_a, &bytes_b]).expect("Failed merge_pdf_bytes");
    let mut reloaded_fast = PdfDocument::load(&fast_merged_bytes).expect("Failed reload fast");
    assert_eq!(reloaded_fast.get_pages().unwrap().len(), 3);
}

#[test]
fn test_reorder_and_delete_pages() {
    let bytes = create_test_multipage_pdf(&["Page 0", "Page 1", "Page 2", "Page 3"]);
    let mut doc = PdfDocument::load(&bytes).expect("Failed to load PDF");

    // Reorder: 3, 2, 1, 0
    reorder_pages(&mut doc, &[3, 2, 1, 0]).expect("Failed to reorder");
    let pages = doc.get_pages().unwrap();
    let p0 = doc.get_page_content_bytes(pages[0]).unwrap();
    let p3 = doc.get_page_content_bytes(pages[3]).unwrap();
    assert!(String::from_utf8_lossy(&p0).contains("Page 3"));
    assert!(String::from_utf8_lossy(&p3).contains("Page 0"));

    // Delete page index 1
    delete_pages(&mut doc, &[1]).expect("Failed to delete page 1");
    let remaining = doc.get_pages().unwrap();
    assert_eq!(remaining.len(), 3);

    // Serialize and reload
    let saved = doc.save_to_vec().unwrap();
    let mut reloaded = PdfDocument::load(&saved).unwrap();
    assert_eq!(reloaded.get_pages().unwrap().len(), 3);
}
