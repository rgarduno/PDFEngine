use pdf_engine_core::cos::{ObjectId, PdfDocument, PdfObject};

/// Creates a minimalist, fully-conformant ISO 32000-1 single-page PDF document.
fn create_test_pdf_bytes() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    // 1 0 obj: Catalog
    let off1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // 2 0 obj: Pages
    let off2 = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>\nendobj\n");

    // 3 0 obj: Page
    let off3 = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 612 792 ] /Contents 4 0 R >>\nendobj\n",
    );

    // 4 0 obj: Content Stream
    let off4 = pdf.len();
    let stream_content = b"BT\n/F1 24 Tf\n100 700 Td\n(Hello ISO 32000 PDF) Tj\nET\n";
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Length {} >>\nstream\n",
            stream_content.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(stream_content);
    pdf.extend_from_slice(b"endstream\nendobj\n");

    // Cross-reference table
    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());

    // Trailer
    pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
    pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());

    pdf
}

#[test]
fn test_pdf_document_load_and_traverse_pages() {
    let pdf_bytes = create_test_pdf_bytes();
    let mut doc = PdfDocument::load(&pdf_bytes).expect("Failed to parse valid PDF");

    // Verify Catalog
    let catalog = doc.catalog().expect("Failed to resolve catalog");
    assert_eq!(
        catalog.get("Type").and_then(|t| t.as_name()),
        Some("Catalog")
    );

    // Verify Pages tree traversal
    let pages = doc.get_pages().expect("Failed to retrieve pages");
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0], ObjectId::new(3));

    // Verify Page Object
    let page_obj = doc.get_object(pages[0]).expect("Failed to get page object");
    let page_dict = page_obj.as_dict().expect("Page must be a dictionary");
    assert_eq!(page_dict.get("Type").and_then(|t| t.as_name()), Some("Page"));

    // Verify Content Stream
    let contents_ref = page_dict
        .get("Contents")
        .and_then(|c| c.as_reference())
        .expect("Page missing /Contents reference");
    let contents_obj = doc.get_object(contents_ref).expect("Failed to get contents");
    match contents_obj {
        PdfObject::Stream(s) => {
            let text = String::from_utf8_lossy(&s.content);
            assert!(text.contains("Hello ISO 32000 PDF"));
        }
        _ => panic!("Contents must be a stream"),
    }
}
