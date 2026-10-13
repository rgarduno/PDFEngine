use crate::cos::{ObjectId, PdfDictionary, PdfDocument, PdfName, PdfObject, PdfStream, Writer};
use crate::layout::Rect;
use crate::ops::{
    delete_pages, duplicate_page, extract_pages, get_page_rotation, insert_blank_page,
    insert_pages, merge_documents, merge_pdf_bytes, page_boxes, reorder_pages, reverse_pages,
    rotate_all_pages, rotate_page, search_document, set_page_boundary, set_page_rotation,
    split_by_ranges, split_document,
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

#[test]
fn test_diff_word_level_lcs() {
    use crate::ops::diff::compute_word_diffs;
    let words_a = vec![
        "El".to_string(),
        "contrato".to_string(),
        "vence".to_string(),
        "en".to_string(),
        "2024".to_string(),
    ];
    let words_b = vec![
        "El".to_string(),
        "contrato".to_string(),
        "vence".to_string(),
        "en".to_string(),
        "2025".to_string(),
    ];

    let (diffs, sim) = compute_word_diffs(&words_a, &words_b, false);
    assert!(sim > 0.7);
    assert_eq!(diffs.len(), 6); // El, contrato, vence, en, 2024 (del), 2025 (add)

    use crate::ops::diff::DiffKind;
    assert_eq!(diffs[0].kind, DiffKind::Unchanged);
    assert_eq!(diffs[4].kind, DiffKind::Deleted);
    assert_eq!(diffs[4].text, "2024");
    assert_eq!(diffs[5].kind, DiffKind::Added);
    assert_eq!(diffs[5].text, "2025");
}

#[test]
fn test_compare_documents_identical() {
    use crate::ops::diff::{compare_documents, DiffOptions};

    let bytes = create_test_multipage_pdf(&["Linea 1 de prueba", "Segunda pagina"]);
    let mut doc_a = PdfDocument::load(&bytes).expect("Failed to load PDF A");
    let mut doc_b = PdfDocument::load(&bytes).expect("Failed to load PDF B");

    let report = compare_documents(&mut doc_a, &mut doc_b, &DiffOptions::default())
        .expect("Failed to compare");
    assert!(report.is_identical);
    assert_eq!(report.summary.base_page_count, 2);
    assert_eq!(report.summary.target_page_count, 2);
    assert_eq!(report.summary.text_additions, 0);
    assert_eq!(report.summary.text_deletions, 0);
    assert_eq!(report.summary.text_modifications, 0);
    assert_eq!(report.summary.total_pages_with_changes, 0);
}

#[test]
fn test_compare_documents_text_modifications() {
    use crate::ops::diff::{compare_documents, DiffKind, DiffOptions};

    let bytes_a = create_test_multipage_pdf(&["Contrato de Servicios 2024"]);
    let bytes_b = create_test_multipage_pdf(&["Contrato de Servicios 2025"]);

    let mut doc_a = PdfDocument::load(&bytes_a).expect("Failed to load PDF A");
    let mut doc_b = PdfDocument::load(&bytes_b).expect("Failed to load PDF B");

    let report = compare_documents(&mut doc_a, &mut doc_b, &DiffOptions::default())
        .expect("Failed to compare");
    assert!(!report.is_identical);
    assert_eq!(report.summary.text_modifications, 1);
    assert_eq!(report.summary.total_pages_with_changes, 1);

    let page_diff = &report.pages[0];
    assert_eq!(page_diff.kind, DiffKind::Modified);
    assert_eq!(page_diff.text_diffs.len(), 1);
    let item = &page_diff.text_diffs[0];
    assert_eq!(item.kind, DiffKind::Modified);
    assert_eq!(
        item.base_text.as_deref(),
        Some("Contrato de Servicios 2024")
    );
    assert_eq!(
        item.target_text.as_deref(),
        Some("Contrato de Servicios 2025")
    );

    // Check JSON serialization
    let json = report.to_json();
    assert!(json.contains("\"is_identical\":false"));
    assert!(json.contains("\"text_modifications\":1"));
    assert!(json.contains("Contrato de Servicios 2024"));
    assert!(json.contains("Contrato de Servicios 2025"));
}

#[test]
fn test_compare_documents_page_count_discrepancy() {
    use crate::ops::diff::{compare_documents, DiffKind, DiffOptions};

    let bytes_a = create_test_multipage_pdf(&["Unica pagina"]);
    let bytes_b = create_test_multipage_pdf(&["Unica pagina", "Nueva pagina anadida"]);

    let mut doc_a = PdfDocument::load(&bytes_a).expect("Failed to load PDF A");
    let mut doc_b = PdfDocument::load(&bytes_b).expect("Failed to load PDF B");

    let report = compare_documents(&mut doc_a, &mut doc_b, &DiffOptions::default())
        .expect("Failed to compare");
    assert!(!report.is_identical);
    assert_eq!(report.summary.base_page_count, 1);
    assert_eq!(report.summary.target_page_count, 2);
    assert_eq!(report.summary.text_additions, 1);

    assert_eq!(report.pages.len(), 2);
    assert_eq!(report.pages[0].kind, DiffKind::Unchanged);
    assert_eq!(report.pages[1].kind, DiffKind::Added);
}

#[test]
fn test_compare_documents_metadata_diff() {
    use crate::ops::diff::{compare_documents, DiffOptions};
    use crate::ops::metadata::{update_metadata, DocumentMetadata};

    let bytes_a = create_test_multipage_pdf(&["Texto"]);
    let bytes_b = create_test_multipage_pdf(&["Texto"]);

    let mut doc_a = PdfDocument::load(&bytes_a).expect("Failed to load PDF A");
    let mut doc_b = PdfDocument::load(&bytes_b).expect("Failed to load PDF B");

    let meta_b = DocumentMetadata {
        title: Some("Titulo Modificado".to_string()),
        author: Some("Autor Revision".to_string()),
        ..Default::default()
    };
    update_metadata(&mut doc_b, &meta_b).expect("Failed to update metadata");

    let report = compare_documents(&mut doc_a, &mut doc_b, &DiffOptions::default())
        .expect("Failed to compare");
    assert!(!report.is_identical);
    assert!(report.summary.metadata_changes >= 1);
    assert!(report.metadata_diffs.iter().any(|m| m.field == "title"));
}

fn content_object_number(doc: &mut PdfDocument, page_index: usize) -> u32 {
    let page_id = doc.get_pages().expect("pages")[page_index];
    let page = doc.get_object(page_id).expect("page");
    page.as_dict()
        .and_then(|dict| dict.get("Contents"))
        .and_then(|contents| contents.as_reference())
        .expect("contents")
        .number
}

fn replace_page_content(doc: &mut PdfDocument, page_index: usize, content: &[u8]) {
    let contents_id = ObjectId::new(content_object_number(doc, page_index));
    let mut stream = match doc.get_object(contents_id).expect("stream") {
        PdfObject::Stream(stream) => stream,
        _ => panic!("page contents are not a stream"),
    };
    stream.content = content.to_vec();
    stream.dict.insert("Length", stream.content.len() as i64);
    doc.set_object(contents_id, PdfObject::Stream(stream));
}

#[test]
fn test_insert_blank_page_at_front_and_append() {
    let bytes = create_test_multipage_pdf(&["Alpha", "Beta"]);
    let mut doc = PdfDocument::load(&bytes).expect("load");
    let landed = insert_blank_page(&mut doc, 0, 200.0, 300.0).expect("blank");
    assert_eq!(landed, 0);
    assert_eq!(doc.get_pages().expect("pages").len(), 3);

    let boxes = page_boxes(&mut doc, 0).expect("boxes");
    assert_eq!(boxes.media, Rect::new(0.0, 0.0, 200.0, 300.0));
    let page_id = doc.get_pages().expect("pages")[0];
    assert!(doc.get_page_content_bytes(page_id).expect("bytes").is_empty());

    let alpha = search_document(&mut doc, "Alpha", false, 10).expect("search");
    assert_eq!(alpha.hits[0].page_index, 1);

    let count = doc.get_pages().expect("pages").len();
    let appended = insert_blank_page(&mut doc, count, 100.0, 100.0).expect("append");
    assert_eq!(appended, count);
    assert_eq!(doc.get_pages().expect("pages").len(), count + 1);

    assert!(insert_blank_page(&mut doc, 0, f64::NAN, 100.0).is_err());
    assert!(insert_blank_page(&mut doc, 99, 100.0, 100.0).is_err());
}

#[test]
fn test_duplicate_page_does_not_share_the_content_stream() {
    let bytes = create_test_multipage_pdf(&["Alpha", "Beta"]);
    let mut doc = PdfDocument::load(&bytes).expect("load");
    let landed = duplicate_page(&mut doc, 0, 1).expect("duplicate");
    assert_eq!(landed, 1);
    assert_eq!(doc.get_pages().expect("pages").len(), 3);
    assert_ne!(
        content_object_number(&mut doc, 0),
        content_object_number(&mut doc, 1)
    );

    let alpha = search_document(&mut doc, "Alpha", false, 10).expect("search");
    let pages: Vec<usize> = alpha.hits.iter().map(|hit| hit.page_index).collect();
    assert_eq!(pages, vec![0, 1]);
    let beta = search_document(&mut doc, "Beta", false, 10).expect("search");
    assert_eq!(beta.hits[0].page_index, 2);
}

#[test]
fn test_insert_pages_at_index_leaves_the_source_unchanged() {
    let mut dest = PdfDocument::load(&create_test_multipage_pdf(&["Alpha", "Beta"])).expect("dest");
    let mut source = PdfDocument::load(&create_test_multipage_pdf(&["Gamma"])).expect("source");
    let inserted = insert_pages(&mut dest, &mut source, &[0], 1).expect("insert");
    assert_eq!(inserted, 1);
    assert_eq!(source.get_pages().expect("source pages").len(), 1);
    assert_eq!(dest.get_pages().expect("dest pages").len(), 3);

    assert_eq!(
        search_document(&mut dest, "Alpha", false, 10)
            .expect("alpha")
            .hits[0]
            .page_index,
        0
    );
    assert_eq!(
        search_document(&mut dest, "Gamma", false, 10)
            .expect("gamma")
            .hits[0]
            .page_index,
        1
    );
    assert_eq!(
        search_document(&mut dest, "Beta", false, 10)
            .expect("beta")
            .hits[0]
            .page_index,
        2
    );

    let repeated = insert_pages(&mut dest, &mut source, &[0, 0], 0).expect_err("repeat");
    assert!(repeated.to_string().contains("repeated"));
    let too_many = vec![0usize; 201];
    let capped = insert_pages(&mut dest, &mut source, &too_many, 0).expect_err("cap");
    assert!(capped.to_string().contains("200"));
    assert!(!capped.to_string().contains("Invalid page"));
}

#[test]
fn test_reverse_pages_and_round_trip_save() {
    let mut doc = PdfDocument::load(&create_test_multipage_pdf(&["Alpha", "Beta"])).expect("load");
    reverse_pages(&mut doc).expect("reverse");
    assert_eq!(
        search_document(&mut doc, "Beta", false, 10)
            .expect("beta")
            .hits[0]
            .page_index,
        0
    );
    assert_eq!(
        search_document(&mut doc, "Alpha", false, 10)
            .expect("alpha")
            .hits[0]
            .page_index,
        1
    );

    let saved = doc.save_to_vec().expect("save");
    let mut loaded = PdfDocument::load(&saved).expect("reload");
    assert_eq!(loaded.get_pages().expect("pages").len(), 2);
    assert_eq!(
        search_document(&mut loaded, "Beta", false, 10)
            .expect("reloaded")
            .hits[0]
            .page_index,
        0
    );
}

#[test]
fn test_page_boxes_set_clear_and_inherit() {
    let mut doc = PdfDocument::empty();
    insert_blank_page(&mut doc, 0, 612.0, 792.0).expect("blank");
    let pages_id = doc.pages_id().expect("pages id");
    let mut pages = match doc.get_object(pages_id).expect("pages") {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("pages root"),
    };
    pages.insert(
        "CropBox",
        vec![
            PdfObject::Real(10.0),
            PdfObject::Real(10.0),
            PdfObject::Real(100.0),
            PdfObject::Real(100.0),
        ],
    );
    doc.set_object(pages_id, PdfObject::Dictionary(pages));

    let inherited = page_boxes(&mut doc, 0).expect("inherited");
    assert_eq!(inherited.crop, Some(Rect::new(10.0, 10.0, 100.0, 100.0)));

    let written = set_page_boundary(&mut doc, 0, "crop", Some(&[72.0, 72.0, 400.0, 500.0]))
        .expect("set crop");
    assert_eq!(written.crop, Some(Rect::new(72.0, 72.0, 400.0, 500.0)));

    let cleared = set_page_boundary(&mut doc, 0, "CropBox", None).expect("clear");
    assert_eq!(cleared.crop, Some(Rect::new(10.0, 10.0, 100.0, 100.0)));

    assert!(set_page_boundary(&mut doc, 0, "crop", Some(&[700.0, 700.0, 800.0, 800.0])).is_err());
    assert!(set_page_boundary(&mut doc, 0, "crop", Some(&[10.0, 10.0, 10.0, 40.0])).is_err());
    assert!(set_page_boundary(&mut doc, 0, "media", None).is_err());
}

#[test]
fn test_search_case_fold_cap_and_skipped_page() {
    let mut doc = PdfDocument::load(&create_test_multipage_pdf(&["Hello Hello", "Beta"]))
        .expect("load");
    let folded = search_document(&mut doc, "hello", true, 10).expect("fold");
    assert_eq!(folded.hits.len(), 2);
    assert_eq!(folded.hits[0].char_start, 0);
    assert_eq!(folded.hits[0].char_end, 5);
    assert!(!folded.truncated);
    assert!(folded.hits[0].snippet.contains("Hello"));

    let capped = search_document(&mut doc, "Hello", false, 1).expect("cap");
    assert_eq!(capped.hits.len(), 1);
    assert!(capped.truncated);
    assert!(search_document(&mut doc, "", false, 10).is_err());

    replace_page_content(&mut doc, 1, b"(");
    let skipped = search_document(&mut doc, "Hello", false, 10).expect("skip");
    assert_eq!(skipped.pages_skipped, vec![1]);
    assert_eq!(skipped.hits.len(), 2);
}
