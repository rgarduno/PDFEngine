use super::*;
use crate::cos::PdfDocument;
use crate::layout::geometry::Rect;

/// Constructs a valid minimal 2-page PDF document for testing annotations.
fn create_test_pdf_two_pages() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    // Object 1: Catalog
    let offset1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages
    let offset2 = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>\nendobj\n");

    // Object 3: Page 1
    let offset3 = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
    );

    // Object 4: Contents Page 1
    let offset4 = pdf.len();
    pdf.extend_from_slice(
        b"4 0 obj\n<< /Length 38 >>\nstream\nq\nBT /F1 12 Tf 72 700 Td (Page 1) Tj ET\nQ\nendstream\nendobj\n",
    );

    // Object 5: Page 2
    let offset5 = pdf.len();
    pdf.extend_from_slice(
        b"5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>\nendobj\n",
    );

    // Object 6: Contents Page 2
    let offset6 = pdf.len();
    pdf.extend_from_slice(
        b"6 0 obj\n<< /Length 38 >>\nstream\nq\nBT /F1 12 Tf 72 700 Td (Page 2) Tj ET\nQ\nendstream\nendobj\n",
    );

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 7\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset1).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset2).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset3).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset4).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset5).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset6).as_bytes());

    pdf.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n");
    pdf.extend_from_slice(format!("{}\n%%EOF", xref_offset).as_bytes());

    pdf
}

#[test]
fn test_highlight_underline_strikeout_creation_and_reading() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).unwrap();

    let hl_rect = Rect::new(72.0, 690.0, 200.0, 715.0);
    let hl_id = add_text_markup(
        &mut doc,
        0,
        AnnotationSubtype::Highlight,
        hl_rect,
        None,
        Some([1.0, 0.9, 0.1]),
        Some(0.5),
        Some("Important Section"),
    )
    .unwrap();

    let ul_rect = Rect::new(72.0, 650.0, 250.0, 665.0);
    let ul_id = add_text_markup(
        &mut doc,
        0,
        AnnotationSubtype::Underline,
        ul_rect,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let so_rect = Rect::new(72.0, 600.0, 180.0, 615.0);
    let so_id = add_text_markup(
        &mut doc,
        0,
        AnnotationSubtype::StrikeOut,
        so_rect,
        None,
        None,
        None,
        Some("Deprecated"),
    )
    .unwrap();

    assert_ne!(hl_id, ul_id);
    assert_ne!(ul_id, so_id);

    let annots = extract_page_annotations(&mut doc, 0).unwrap();
    assert_eq!(annots.len(), 3);

    let hl = annots.iter().find(|a| a.id == hl_id).unwrap();
    assert_eq!(hl.subtype, AnnotationSubtype::Highlight);
    assert_eq!(hl.contents.as_deref(), Some("Important Section"));
    assert_eq!(hl.color, Some([1.0, 0.9, 0.1]));
    assert!((hl.opacity - 0.5).abs() < 0.01);

    let ul = annots.iter().find(|a| a.id == ul_id).unwrap();
    assert_eq!(ul.subtype, AnnotationSubtype::Underline);

    let so = annots.iter().find(|a| a.id == so_id).unwrap();
    assert_eq!(so.subtype, AnnotationSubtype::StrikeOut);
    assert_eq!(so.contents.as_deref(), Some("Deprecated"));
}

#[test]
fn test_link_annotations_uri_and_goto() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).unwrap();

    let uri_rect = Rect::new(100.0, 500.0, 300.0, 520.0);
    let uri_id = add_link_uri(
        &mut doc,
        0,
        uri_rect,
        "https://github.com/rgarduno/PDFEngine",
        true,
    )
    .unwrap();

    let goto_rect = Rect::new(100.0, 450.0, 250.0, 470.0);
    let goto_id = add_link_goto(&mut doc, 0, goto_rect, 1).unwrap();

    let annots = extract_page_annotations(&mut doc, 0).unwrap();
    assert_eq!(annots.len(), 2);

    let uri_annot = annots.iter().find(|a| a.id == uri_id).unwrap();
    assert_eq!(uri_annot.subtype, AnnotationSubtype::Link);
    assert_eq!(
        uri_annot.link_action,
        Some(LinkAction::Uri(
            "https://github.com/rgarduno/PDFEngine".into()
        ))
    );

    let goto_annot = annots.iter().find(|a| a.id == goto_id).unwrap();
    assert_eq!(goto_annot.subtype, AnnotationSubtype::Link);
    assert_eq!(goto_annot.link_action, Some(LinkAction::GoTo(1)));
}

#[test]
fn test_link_uri_rejects_non_web_schemes() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).unwrap();
    let rect = Rect::new(100.0, 500.0, 300.0, 520.0);
    let rejected = [
        "javascript:example",
        "file:example",
        "mailto:person@example.com",
        "http:/single",
        " https://example.com",
        "https://",
    ];
    for uri in rejected {
        let err = add_link_uri(&mut doc, 0, rect, uri, false).unwrap_err();
        assert!(
            err.to_string().contains("http or https"),
            "{uri} produced {err}"
        );
    }

    let annots = extract_page_annotations(&mut doc, 0).unwrap();
    assert!(annots.is_empty());

    let accepted = add_link_uri(&mut doc, 0, rect, "HTTPS://example.com/terms", false).unwrap();
    let annots = extract_page_annotations(&mut doc, 0).unwrap();
    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].id, accepted);
    assert_eq!(
        annots[0].link_action,
        Some(LinkAction::Uri("HTTPS://example.com/terms".into()))
    );
}

#[test]
fn test_stamp_annotation_creation_and_reading() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).unwrap();

    let stamp_rect = Rect::new(350.0, 700.0, 520.0, 750.0);
    let stamp_id = add_stamp(
        &mut doc,
        0,
        StampType::Approved,
        Some(stamp_rect),
        None,
        None,
        Some("2026-10-02"),
    )
    .unwrap();

    let annots = extract_page_annotations(&mut doc, 0).unwrap();
    assert_eq!(annots.len(), 1);

    let stamp = &annots[0];
    assert_eq!(stamp.id, stamp_id);
    assert_eq!(stamp.subtype, AnnotationSubtype::Stamp);
    assert_eq!(stamp.contents.as_deref(), Some("APPROVED"));
    assert_eq!(stamp.date_str.as_deref(), Some("2026-10-02"));
    assert_eq!(stamp.stamp_type, Some(StampType::Approved));
}

#[test]
fn test_delete_annotation() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).unwrap();

    let stamp_rect = Rect::new(350.0, 700.0, 520.0, 750.0);
    let stamp_id = add_stamp(
        &mut doc,
        0,
        StampType::Confidential,
        Some(stamp_rect),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(extract_page_annotations(&mut doc, 0).unwrap().len(), 1);

    let deleted = delete_annotation(&mut doc, 0, stamp_id).unwrap();
    assert!(deleted);

    assert_eq!(extract_page_annotations(&mut doc, 0).unwrap().len(), 0);
    assert!(!doc.objects.contains_key(&stamp_id));
}

#[test]
fn test_flatten_annotations_to_contents() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).unwrap();

    let hl_rect = Rect::new(72.0, 690.0, 200.0, 715.0);
    add_text_markup(
        &mut doc,
        0,
        AnnotationSubtype::Highlight,
        hl_rect,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let stamp_rect = Rect::new(350.0, 700.0, 520.0, 750.0);
    add_stamp(
        &mut doc,
        0,
        StampType::Approved,
        Some(stamp_rect),
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(extract_page_annotations(&mut doc, 0).unwrap().len(), 2);

    let flattened = flatten_annotations(&mut doc, Some(0)).unwrap();
    assert_eq!(flattened, 2);

    // Annotations array should now be empty of visual markups
    assert_eq!(extract_page_annotations(&mut doc, 0).unwrap().len(), 0);

    // Page 1 contents should now contain the flattened operations
    let pages = doc.get_pages().unwrap();
    let plain = doc.get_page_content_bytes(pages[0]).unwrap();
    let text = String::from_utf8_lossy(&plain);
    assert!(text.contains("Flattened Annotations"));
    assert!(text.contains("/GS_HL gs"));
    assert!(text.contains("APPROVED"));
}
