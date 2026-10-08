//! Comprehensive unit tests for PDF Redaction and Content Sanitization.

use super::*;
use crate::fonts::FontMetrics;
use crate::layout::geometry::Rect;
use crate::stream::parser::{build_ast_from_operations, serialize_ast, ContentStreamTokenizer};

#[test]
fn test_pattern_detection_email() {
    let sample = "Contact us at contact@company.org or support@domain.co.uk for inquiries.";
    let matches = find_emails(sample);
    assert_eq!(matches.len(), 2);
    assert_eq!(&sample[matches[0].0..matches[0].1], "contact@company.org");
    assert_eq!(&sample[matches[1].0..matches[1].1], "support@domain.co.uk");
}

#[test]
fn test_pattern_detection_ssn() {
    let sample = "Customer record SSN: 123-45-6789 and secondary 987-65-4321 confirmed.";
    let matches = find_ssn(sample);
    assert_eq!(matches.len(), 2);
    assert_eq!(&sample[matches[0].0..matches[0].1], "123-45-6789");
    assert_eq!(&sample[matches[1].0..matches[1].1], "987-65-4321");
}

#[test]
fn test_pattern_detection_credit_card() {
    // Standard valid Luhn card: 4532-0150-1234-5678 (4532015012345678)
    // 4*2=8, 5, 3*2=6, 2, 0*2=0, 1, 5*2=1, 0, 1*2=2, 2, 3*2=6, 4, 5*2=1, 6, 7*2=5, 8 => sum=55 (wait, let's construct true Luhn)
    // Let's test 49927398716 or 4532 0150 1234 5678 or 0000 0000 0000 0000 (all 0 is sum 0 % 10 == 0)
    let sample = "Paid with card: 0000-0000-0000-0000 on terminal.";
    let matches = find_credit_cards(sample);
    assert_eq!(matches.len(), 1);
    assert_eq!(&sample[matches[0].0..matches[0].1], "0000-0000-0000-0000");
}

#[test]
fn test_pattern_detection_rfc_and_curp() {
    let sample = "Contribuyente RFC: GARM850101XYZ y CURP: GARM850101HDFRRN01 validado.";
    let rfc_matches = find_rfc(sample);
    assert_eq!(rfc_matches.len(), 1);
    assert_eq!(&sample[rfc_matches[0].0..rfc_matches[0].1], "GARM850101XYZ");

    let curp_matches = find_curp(sample);
    assert_eq!(curp_matches.len(), 1);
    assert_eq!(
        &sample[curp_matches[0].0..curp_matches[0].1],
        "GARM850101HDFRRN01"
    );
}

#[test]
fn test_pattern_detection_substring() {
    let sample = "Project Manhattan is Top Secret. All top secret files must be sealed.";
    let matches = find_substring(sample, "Top Secret", false);
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_surgical_ast_redaction_text_purged() {
    // Construct a content stream containing sensitive text:
    // (Notice width: font_size=12, char advance approx 6pt)
    // "Secret: 123-45-6789 OK"
    // At x=100, y=500:
    // "Secret: " (8 chars ~ 48pt) -> x from 100 to 148
    // "123-45-6789" (11 chars ~ 66pt) -> x from 148 to 214
    // " OK" (3 chars ~ 18pt) -> x from 214 to 232
    let stream = b"BT\n/F1 12 Tf\n1 0 0 1 100 500 Tm\n(Secret: 123-45-6789 OK) Tj\nET\n";
    let mut tokenizer = ContentStreamTokenizer::new(stream);
    let ops = tokenizer.tokenize_all().unwrap();
    let mut ast = build_ast_from_operations(ops);

    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);

    // Define a redaction rectangle precisely over "123-45-6789"
    // glyphs for "123-45-6789" start around x=148 and end around x=214
    let redact_rect = Rect::new(145.0, 495.0, 216.0, 515.0);
    let redaction = RedactionRect::new(redact_rect).with_overlay_text("[REDACTADO]", None);

    let summary = apply_redaction_to_ast(&mut ast, &[redaction], &metrics).unwrap();

    assert!(summary.purged_glyphs_count >= 10);
    assert_eq!(summary.blackout_boxes_count, 1);

    // Serialize mutated stream
    let output_bytes = serialize_ast(&ast);
    let output_str = String::from_utf8_lossy(&output_bytes);

    // CRITICAL ASSERTION: The sensitive string MUST NOT exist in output bytes!
    assert!(
        !output_str.contains("123-45-6789"),
        "Sensitive text leaked in output bytes!"
    );

    // Non-redacted text MUST be preserved!
    assert!(output_str.contains("(Secret: ) Tj") || output_str.contains("Secret:"));
    assert!(output_str.contains("( OK) Tj") || output_str.contains("OK"));

    // Blackout box vector operators MUST be present
    assert!(output_str.contains("re\nf"));
    assert!(output_str.contains("([REDACTADO]) Tj"));
}

fn create_test_pdf_with_email() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    // Object 1: Catalog
    let offset1 = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages
    let offset2 = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    // Object 3: Page 1 with Annotations
    let offset3 = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [5 0 R] >>\nendobj\n",
    );

    // Object 4: Contents
    let offset4 = pdf.len();
    let stream_content = b"BT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Client Email: confidential@corp.com Verified) Tj\nET\n";
    pdf.extend_from_slice(
        format!("4 0 obj\n<< /Length {} >>\nstream\n", stream_content.len()).as_bytes(),
    );
    pdf.extend_from_slice(stream_content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    // Object 5: Link Annotation
    let offset5 = pdf.len();
    pdf.extend_from_slice(
        b"5 0 obj\n<< /Type /Annot /Subtype /Link /Rect [140 695 270 715] >>\nendobj\n",
    );

    // Object 6: Info
    let offset6 = pdf.len();
    pdf.extend_from_slice(b"6 0 obj\n<< /Author (Secret Agent) /Title (Classified) >>\nendobj\n");

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 7\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset1).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset2).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset3).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset4).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset5).as_bytes());
    pdf.extend_from_slice(format!("{:010} 00000 n \n", offset6).as_bytes());

    pdf.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R >>\nstartxref\n");
    pdf.extend_from_slice(format!("{}\n%%EOF", xref_offset).as_bytes());

    pdf
}

#[test]
fn test_full_document_redaction_workflow() {
    let pdf_bytes = create_test_pdf_with_email();
    let mut doc = crate::cos::PdfDocument::load(&pdf_bytes).unwrap();
    let page_id = doc.get_pages().unwrap()[0];

    // Run Pattern Redaction targeting Email
    let mut config = RedactionConfig::default();
    config.scrub_metadata = true;
    config.prune_annotations = true;

    let summaries =
        redact_document_pattern(&mut doc, &RedactionPattern::Email, None, &config).unwrap();

    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].purged_glyphs_count > 0);
    assert_eq!(summaries[0].pruned_annotations_count, 1);

    // Verify content stream bytes
    let updated_bytes = doc.get_page_content_bytes(page_id).unwrap();
    let updated_str = String::from_utf8_lossy(&updated_bytes);
    assert!(!updated_str.contains("confidential@corp.com"));
    assert!(updated_str.contains("Client Email:"));
    assert!(updated_str.contains("Verified"));

    // Verify annotation was pruned from page dictionary
    let updated_page = doc.get_object(page_id).unwrap().as_dict().unwrap().clone();
    if let Some(crate::cos::PdfObject::Array(annots)) = updated_page.get("Annots") {
        assert_eq!(annots.len(), 0);
    }

    // Verify metadata was scrubbed
    let info_id = doc
        .xref
        .trailer
        .get("Info")
        .and_then(|i| i.as_reference())
        .unwrap();
    let updated_info = doc.get_object(info_id).unwrap().as_dict().unwrap().clone();
    assert!(!updated_info.contains_key("Author"));
    assert!(!updated_info.contains_key("Title"));
    assert_eq!(
        updated_info
            .get("Producer")
            .and_then(|p| p.as_string())
            .map(|s| s.to_string_lossy()),
        Some("PDFEngine Sanitizer".to_string())
    );
}

#[test]
fn redaction_strips_alternate_text_on_the_rewritten_page() {
    let stream = b"/Span << /ActualText (hidden-actual) /Alt (hidden-alt) /E (hidden-expansion) /MCID 7 >> BDC\nBT\n/F1 12 Tf\n1 0 0 1 72 700 Tm\n(Visible clause) Tj\nET\nEMC\n";
    let mut tokenizer = ContentStreamTokenizer::new(stream);
    let ops = tokenizer.tokenize_all().unwrap();
    let mut ast = build_ast_from_operations(ops);
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let redaction = RedactionRect::new(Rect::new(70.0, 690.0, 220.0, 720.0));

    apply_redaction_to_ast(&mut ast, &[redaction], &metrics).unwrap();

    let serialized = serialize_ast(&ast);
    let output = String::from_utf8_lossy(&serialized);
    assert!(!output.contains("hidden-actual"));
    assert!(!output.contains("hidden-alt"));
    assert!(!output.contains("hidden-expansion"));
    assert!(!output.contains("ActualText"));
    assert!(output.contains("MCID"));
}

#[test]
fn redaction_drops_page_metadata_and_keeps_document_info() {
    use crate::cos::object::{PdfDictionary, PdfObject};
    use crate::cos::PdfDocument;

    let mut doc = PdfDocument::load(&create_test_pdf_with_email()).unwrap();
    let page_id = doc.get_pages().unwrap()[0];
    let meta_id = doc.alloc_object_id();
    let mut meta = PdfDictionary::new();
    meta.insert("Type", PdfObject::Name("Metadata".into()));
    doc.set_object(meta_id, PdfObject::Dictionary(meta));

    let mut page = match doc.get_object(page_id).unwrap() {
        PdfObject::Dictionary(dict) => dict,
        _ => panic!("page object"),
    };
    page.insert("Metadata", PdfObject::Reference(meta_id));
    doc.set_object(page_id, PdfObject::Dictionary(page));

    let config = RedactionConfig::default();
    assert!(!config.scrub_metadata);
    redact_document_rectangles(
        &mut doc,
        0,
        &[Rect::new(70.0, 690.0, 400.0, 720.0)],
        &config,
    )
    .unwrap();

    let page = doc.get_object(page_id).unwrap().as_dict().unwrap().clone();
    assert!(!page.contains_key("Metadata"));

    let info_id = doc
        .xref
        .trailer
        .get("Info")
        .and_then(|item| item.as_reference())
        .unwrap();
    let info = doc.get_object(info_id).unwrap().as_dict().unwrap().clone();
    assert!(info.contains_key("Author"));
}
