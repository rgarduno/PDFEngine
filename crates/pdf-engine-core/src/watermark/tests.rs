use super::*;
use crate::cos::PdfDocument;
use crate::images::png::encode_png;

fn create_test_pdf_two_pages() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    let off1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    let off2 = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>\nendobj\n");

    let off3 = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n",
    );

    let off4 = pdf.len();
    pdf.extend_from_slice(
        b"4 0 obj\n<< /Length 38 >>\nstream\nq\nBT /F1 12 Tf 72 700 Td (Page 1 Content) Tj ET\nQ\nendstream\nendobj\n",
    );

    let off5 = pdf.len();
    pdf.extend_from_slice(
        b"5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>\nendobj\n",
    );

    let off6 = pdf.len();
    pdf.extend_from_slice(
        b"6 0 obj\n<< /Length 38 >>\nstream\nq\nBT /F1 12 Tf 72 700 Td (Page 2 Content) Tj ET\nQ\nendstream\nendobj\n",
    );

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 7\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off1).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off2).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off3).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off4).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off5).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", off6).as_bytes());

    pdf.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R >>\n");
    pdf.extend_from_slice(format!("startxref\n{}\n%%EOF\n", xref_offset).as_bytes());

    pdf
}

#[test]
fn test_apply_pagination_default() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).expect("Load test PDF");

    let config = PaginationConfig::default();
    let count = apply_pagination(&mut doc, &config).expect("Apply pagination");
    assert_eq!(count, 2);

    let pages = doc.get_pages().expect("Get pages");
    let content_p1 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[0]).unwrap()).to_string();
    let content_p2 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[1]).unwrap()).to_string();

    assert!(content_p1.contains("Página 1 de 2"));
    assert!(content_p2.contains("Página 2 de 2"));
    assert!(content_p1.contains("/F_PAG"));
    assert!(content_p2.contains("/F_PAG"));
}

#[test]
fn test_apply_pagination_skip_first_page() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).expect("Load test PDF");

    let config = PaginationConfig {
        format: "Sheet {page} of {total}".to_string(),
        position: PaginationPosition::BottomRight,
        font_size: 10.0,
        color: [0.2, 0.2, 0.2],
        margin: 40.0,
        start_page_num: 1,
        skip_first_page: true,
        page_indices: None,
    };

    let count = apply_pagination(&mut doc, &config).expect("Apply pagination with skip");
    assert_eq!(count, 1);

    let pages = doc.get_pages().expect("Get pages");
    let content_p1 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[0]).unwrap()).to_string();
    let content_p2 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[1]).unwrap()).to_string();

    assert!(!content_p1.contains("Sheet"));
    assert!(content_p2.contains("Sheet 1 of 2"));
}

#[test]
fn test_apply_text_watermark_background() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).expect("Load test PDF");

    let config = TextWatermarkConfig {
        text: "CONFIDENCIAL".to_string(),
        font_size: 60.0,
        color: [0.9, 0.1, 0.1],
        opacity: 0.20,
        rotation_degrees: 45.0,
        placement: WatermarkPlacement::Background,
        page_indices: None,
    };

    let count = apply_text_watermark(&mut doc, &config).expect("Apply text watermark");
    assert_eq!(count, 2);

    let pages = doc.get_pages().expect("Get pages");
    let content_p1 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[0]).unwrap()).to_string();

    assert!(content_p1.contains("/GS_WM gs"));
    assert!(content_p1.contains("/F_WM"));
    assert!(content_p1.contains("CONFIDENCIAL"));

    // Verify background placement: watermark appears before original page content
    let wm_pos = content_p1.find("CONFIDENCIAL").unwrap();
    let orig_pos = content_p1.find("Page 1 Content").unwrap();
    assert!(wm_pos < orig_pos, "Background watermark must be prepended before original page content");
}

#[test]
fn test_apply_text_watermark_foreground() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).expect("Load test PDF");

    let config = TextWatermarkConfig {
        text: "DRAFT".to_string(),
        font_size: 48.0,
        color: [0.5, 0.5, 0.5],
        opacity: 0.35,
        rotation_degrees: 0.0,
        placement: WatermarkPlacement::Foreground,
        page_indices: Some(vec![1]),
    };

    let count = apply_text_watermark(&mut doc, &config).expect("Apply foreground watermark");
    assert_eq!(count, 1);

    let pages = doc.get_pages().expect("Get pages");
    let content_p1 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[0]).unwrap()).to_string();
    let content_p2 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[1]).unwrap()).to_string();

    assert!(!content_p1.contains("DRAFT"));
    assert!(content_p2.contains("DRAFT"));

    // Verify foreground placement: watermark appears after original page content
    let wm_pos = content_p2.find("DRAFT").unwrap();
    let orig_pos = content_p2.find("Page 2 Content").unwrap();
    assert!(wm_pos > orig_pos, "Foreground watermark must be appended after original page content");
}

#[test]
fn test_apply_image_watermark() {
    let pdf_bytes = create_test_pdf_two_pages();
    let mut doc = PdfDocument::load(&pdf_bytes).expect("Load test PDF");

    // Synthesize a 2x2 RGB PNG image
    let raw_pixels = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let png_bytes = encode_png(2, 2, &raw_pixels, true).expect("Encode PNG");

    let config = ImageWatermarkConfig {
        image_bytes: png_bytes,
        width: Some(200.0),
        height: Some(200.0),
        opacity: 0.30,
        rotation_degrees: 45.0,
        placement: WatermarkPlacement::Background,
        page_indices: None,
    };

    let count = apply_image_watermark(&mut doc, &config).expect("Apply image watermark");
    assert_eq!(count, 2);

    let pages = doc.get_pages().expect("Get pages");
    let content_p1 = String::from_utf8_lossy(&doc.get_page_content_bytes(pages[0]).unwrap()).to_string();

    assert!(content_p1.contains("/WM_IMG Do"));
    assert!(content_p1.contains("/GS_WM gs"));
}
